use std::{
    collections::{HashMap, LinkedList},
    future::{ready, Ready},
    pin::Pin,
    sync::{Arc, Mutex},
};

use actix_web::{
    body::{BodySize, BoxBody, EitherBody, MessageBody},
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::{
        header::{CacheControl, HeaderMap},
        StatusCode,
    },
    Error, HttpRequest, HttpResponse,
};

use futures_util::future::LocalBoxFuture;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

// The header to use. Defaults to 'Idempotency-Key' as defined in this IETF memo:
//
// https://www.ietf.org/archive/id/draft-ietf-httpapi-idempotency-key-header-01.html
const HEADER_KEY: &str = "Idempotency-Key";

pub struct Idempotency;

impl<S, B> Transform<S, ServiceRequest> for Idempotency
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;

    type Error = Error;

    type Transform = IdempotencyMiddleware<S>;

    type InitError = ();

    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(IdempotencyMiddleware {
            service,
            queue: Arc::new(Mutex::new(LinkedList::new())),
            response_cache: Arc::new(Mutex::new(HashMap::new())),
        }))
    }
}

#[derive(Hash)]
pub struct CacheElement {
    response: Vec<u8>,
    headers: Vec<(String, String)>,
    statuscode: StatusCode,
    created_at: DateTime<Utc>,
}

pub struct IdempotencyMiddleware<S> {
    service: S,
    queue: Arc<Mutex<LinkedList<Uuid>>>,
    response_cache: Arc<Mutex<HashMap<Uuid, CacheElement>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum IdempotencyError {
    Missing,
    Malformed,
    #[serde(rename = "ALREADY_EXISTS")]
    AlreadyExists,
}

#[derive(Serialize)]
struct IdempotencyErrorWrapper {
    error: IdempotencyError,
}

impl<S, B> Service<ServiceRequest> for IdempotencyMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        println!("Hi from start. You requested: {}", req.path());

        if !req.headers().contains_key(HEADER_KEY) {
            let (http_request, _payload) = req.into_parts();
            return Box::pin(async {
                Ok(ServiceResponse::new(
                    http_request,
                    //syntax magic to force the compiler to transform into `HttpResponse`
                    Into::<HttpResponse>::into(IdempotencyError::Missing).map_into_right_body(),
                ))
            });
        }

        // unwrap/expect is safe as we previously checked for the key existing
        let token = req
            .headers()
            .get(HEADER_KEY)
            .expect("Couldn't extract idempotency key!");

        let token = match Uuid::try_from(token.to_str().unwrap()) {
            Ok(x) => x,

            //token is not a valid Uuid token
            Err(_) => {
                let (http_request, _payload) = req.into_parts();
                return Box::pin(async {
                    Ok(ServiceResponse::new(
                        http_request,
                        //syntax magic to force the compiler to transform into `HttpResponse`
                        Into::<HttpResponse>::into(IdempotencyError::Malformed)
                            .map_into_right_body(),
                    ))
                });
            }
        };

        // we've successfully verified the key

        dbg!(&self.queue);

        let fut = self.service.call(req);

        Box::pin(async move {
            let res = fut.await?;

            self.queue.lock().unwrap().push_back(token.clone());

            let cached_response = res.response().body();

            let x = TryInto::<Vec<u8>>::try_into(cached_response);

            self.response_cache
                .lock()
                .unwrap()
                .insert(token, cached_response);

            println!("Hi from response");

            Ok(res.map_into_left_body())
        })
    }
}

impl From<HttpResponse> for CacheElement {
    fn from(value: HttpResponse) -> Self {
        let headers = value
            .headers()
            .into_iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    String::from_utf8(v.as_bytes().into()).expect("couldn't parse bytes to utf8"),
                )
            })
            .collect();

        // extract response bytes
        let response = match value.body().size() {
            BodySize::None => None,
            BodySize::Sized(_) => {
                let bytes = value
                    .body()
                    .try_into_bytes()
                    .expect("couldn't parse body into bytes");
                // transform from `actix_web::web::Bytes` to `Vec<u8>`
                let bytes = Into::<Vec<u8>>::into(bytes);
                Some(bytes)
            }
            // can streamed responses collected into a 'static' vec?
            BodySize::Stream => {
                let bytes = value
                    .body()
                    .try_into_bytes()
                    .expect("couldn't parse streaming body into bytes");
                let bytes = Into::<Vec<u8>>::into(bytes);
                Some(bytes)
            }
        };

        Self {
            response,
            headers,
            statuscode: value.status(),
            created_at: Utc::now(),
        }
    }
}

impl Into<HttpResponse> for IdempotencyError {
    fn into(self) -> HttpResponse {
        let status = match self {
            Self::Missing | Self::Malformed => StatusCode::BAD_REQUEST,
            Self::AlreadyExists => StatusCode::CONFLICT,
        };

        HttpResponse::build(status).json(&IdempotencyErrorWrapper { error: self })
    }
}
