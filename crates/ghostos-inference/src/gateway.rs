use core::str;

use crate::{
    Error,
    protocol::{
        CompletionRequest, CompletionResponse, GrpcCodec, ModelName, OpenAiCodec, OpenAiRequest,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentType {
    Grpc,
    Json,
    ServerSentEvents,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayResponse {
    pub bytes_written: usize,
    pub content_type: ContentType,
    pub streaming: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionResult {
    pub id: u64,
    pub created_at: u64,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub bytes_written: usize,
    pub finished: bool,
}

/// Model execution boundary used by both wire protocols.
///
/// The executor writes only generated UTF-8 into `text`. It may use the
/// cluster inference service to reserve mirrored KV memory and checkpoint the
/// request before returning.
pub trait InferenceExecutor {
    fn model_count(&self) -> usize;
    fn model_at(&self, index: usize) -> Option<ModelName>;

    fn complete(
        &mut self,
        request: CompletionRequest<'_>,
        text: &mut [u8],
    ) -> Result<ExecutionResult, Error>;
}

/// Transport-neutral Ring 3 gateway.
///
/// `ghostos-netd` or the future `ghostos-http` service supplies request bodies and
/// sends the returned bytes. This layer owns protocol validation and keeps the
/// inference executor independent from HTTP, HTTP/2, and gRPC transports.
pub struct InferenceGateway<E> {
    executor: E,
}

impl<E: InferenceExecutor> InferenceGateway<E> {
    pub const fn new(executor: E) -> Self {
        Self { executor }
    }

    pub const fn executor(&self) -> &E {
        &self.executor
    }

    pub fn executor_mut(&mut self) -> &mut E {
        &mut self.executor
    }

    pub fn into_executor(self) -> E {
        self.executor
    }

    pub fn handle_openai(
        &mut self,
        path: &str,
        body: &[u8],
        text: &mut [u8],
        destination: &mut [u8],
    ) -> Result<GatewayResponse, Error> {
        match OpenAiCodec::decode(path, body)? {
            OpenAiRequest::ListModels => {
                let count = self.executor.model_count();
                let models = (0..count).filter_map(|index| self.executor.model_at(index));
                let bytes_written = OpenAiCodec::encode_model_list(models, destination)?;
                Ok(GatewayResponse {
                    bytes_written,
                    content_type: ContentType::Json,
                    streaming: false,
                })
            }
            OpenAiRequest::Complete(request) => {
                let model = ModelName::new(request.model)?;
                let stream = request.stream;
                let result = self.executor.complete(request, text)?;
                let generated = str::from_utf8(
                    text.get(..result.bytes_written)
                        .ok_or(Error::InvalidRequest)?,
                )
                .map_err(|_| Error::InvalidRequest)?;
                let response = CompletionResponse {
                    id: result.id,
                    model: model.as_str(),
                    text: generated,
                    kind: request.kind,
                    created_at: result.created_at,
                    prompt_tokens: result.prompt_tokens,
                    completion_tokens: result.completion_tokens,
                    finished: result.finished,
                };
                let bytes_written = if stream {
                    OpenAiCodec::encode_sse_chunk(response, destination)?
                } else {
                    OpenAiCodec::encode_completion(response, destination)?
                };
                Ok(GatewayResponse {
                    bytes_written,
                    content_type: if stream {
                        ContentType::ServerSentEvents
                    } else {
                        ContentType::Json
                    },
                    streaming: stream && !result.finished,
                })
            }
        }
    }

    pub fn handle_grpc(
        &mut self,
        frame: &[u8],
        text: &mut [u8],
        destination: &mut [u8],
    ) -> Result<GatewayResponse, Error> {
        let request = GrpcCodec::decode(frame)?;
        let model = ModelName::new(request.model)?;
        let stream = request.stream;
        let result = self.executor.complete(request, text)?;
        let generated = str::from_utf8(
            text.get(..result.bytes_written)
                .ok_or(Error::InvalidRequest)?,
        )
        .map_err(|_| Error::InvalidRequest)?;
        let bytes_written = GrpcCodec::encode(
            CompletionResponse {
                id: result.id,
                model: model.as_str(),
                text: generated,
                kind: request.kind,
                created_at: result.created_at,
                prompt_tokens: result.prompt_tokens,
                completion_tokens: result.completion_tokens,
                finished: result.finished,
            },
            destination,
        )?;
        Ok(GatewayResponse {
            bytes_written,
            content_type: ContentType::Grpc,
            streaming: stream && !result.finished,
        })
    }
}
