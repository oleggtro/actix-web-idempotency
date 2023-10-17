use std::{
    collections::{HashMap, LinkedList},
    future::{ready, Ready},
};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse,
};

use futures_util::future::LocalBoxFuture;

use chrono::{DateTime, Utc};
use uuid::Uuid;

pub struct Idempotency;

impl<S, B> Transform<S, ServiceRequest> for Idempotency
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;

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

impl<S, B> Service<ServiceRequest> for IdempotencyMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        println!("Hi from start. You requested: {}", req.path());

        if !req.headers().contains_key("Idempotency-Key") {
            return ServiceResponse::new(req, HttpResponse());
        }

        let fut = self.service.call(req);

        Box::pin(async move {
            let res = fut.await?;

            println!("Hi from response");
            Ok(res)
        })
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
