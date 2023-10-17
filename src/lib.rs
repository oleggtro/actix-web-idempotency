use std::{
    collections::{HashMap, LinkedList},
    future::{ready, Ready},
    pin::Pin,
};

use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::StatusCode,
    Error, HttpRequest, HttpResponse,
};

use futures_util::future::LocalBoxFuture;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

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
        ready(Ok(IdempotencyMiddleware { service }))
    }
}

pub struct CacheElement {
    token: Uuid,
    response: String,
    created_at: DateTime<Utc>,
}

pub struct IdempotencyMiddleware<S> {
    service: S,
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

        // unwrap/expecrt is safe as we previously checked for the key existing
        let token = req
            .headers()
            .get(HEADER_KEY)
            .expect("Couldn't extract idempotency key!");

        let token = Uuid::try_from(token.to_str().unwrap());

        //token is not a valid Uuid token
        if let Err(_) = token {
            let (http_request, _payload) = req.into_parts();
            return Box::pin(async {
                Ok(ServiceResponse::new(
                    http_request,
                    //syntax magic to force the compiler to transform into `HttpResponse`
                    Into::<HttpResponse>::into(IdempotencyError::Malformed).map_into_right_body(),
                ))
            });
        }

        let fut = self.service.call(req);

        Box::pin(async move {
            let res = fut.await?;

            println!("Hi from response");

            Ok(res.map_into_left_body())
        })
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

pub fn add(left: usize, right: usize) -> usize {
    left + right
}
// https://actix.rs/docs/middleware/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
