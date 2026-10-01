//! Response types returned by the networking client.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::Stream;
use reqwest::{StatusCode, header::HeaderMap};

use crate::error::RequestError;

type BoxedResponseStream = Pin<Box<dyn Stream<Item = Result<Bytes, RequestError>> + Send>>;

/// A stream of response body chunks.
///
/// Each item represents one transport-provided chunk. The stream applies
/// backpressure to the underlying HTTP response and may yield a transport
/// error after response headers have already been delivered.
pub struct ResponseBody {
    inner: BoxedResponseStream,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl std::fmt::Debug for ResponseBody {
    /// Formats the stream without attempting to consume it.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResponseBody")
            .finish_non_exhaustive()
    }
}

impl Stream for ResponseBody {
    type Item = Result<Bytes, RequestError>;

    /// Polls the next response body chunk.
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(context)
    }
}

impl ResponseBody {
    pub(crate) fn from_stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, RequestError>> + Send + 'static,
    {
        Self {
            inner: Box::pin(stream),
            permit: None,
        }
    }

    pub(crate) fn once(body: Vec<u8>) -> Self {
        Self::from_stream(futures_util::stream::once(
            async move { Ok(Bytes::from(body)) },
        ))
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        BoxedResponseStream,
        Option<tokio::sync::OwnedSemaphorePermit>,
    ) {
        (self.inner, self.permit)
    }

    pub(crate) fn attach_permit(&mut self, permit: tokio::sync::OwnedSemaphorePermit) {
        self.permit = Some(permit);
    }
}

/// A fully buffered HTTP response returned by the networking client.
#[derive(Clone, Debug)]
pub struct Response {
    /// HTTP status code returned by the server.
    pub status: StatusCode,
    /// Response headers.
    pub headers: HeaderMap,
    /// Final URL after redirects.
    pub url: String,
    /// Raw response body bytes.
    pub body: Vec<u8>,
    /// Whether this response was served from the local cache.
    pub from_cache: bool,
}

/// Response metadata and a lazily consumed HTTP response body.
///
/// Headers are available as soon as the server response is received. The body
/// remains attached to the network transport and is consumed by polling
/// [`StreamingResponse::body`].
pub struct StreamingResponse {
    /// HTTP status code returned by the server.
    pub status: StatusCode,
    /// Response headers.
    pub headers: HeaderMap,
    /// Final URL after redirects.
    pub url: String,
    /// Body chunks delivered by the network transport.
    pub body: ResponseBody,
    /// Whether this response was served from the local cache.
    pub from_cache: bool,
}

impl std::fmt::Debug for StreamingResponse {
    /// Formats response metadata without consuming the body.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StreamingResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("url", &self.url)
            .field("body", &self.body)
            .field("from_cache", &self.from_cache)
            .finish_non_exhaustive()
    }
}
