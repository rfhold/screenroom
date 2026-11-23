// @generated
/// Generated client implementations.
pub mod operator_service_client {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use tonic::codegen::*;
    use tonic::codegen::http::Uri;
    /** Administrative operations for trusted deployments. Selectors do not authenticate callers.
*/
    #[derive(Debug, Clone)]
    pub struct OperatorServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl<T> OperatorServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + std::marker::Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> OperatorServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<
                    <T as tonic::client::GrpcService<tonic::body::Body>>::ResponseBody,
                >,
            >,
            <T as tonic::codegen::Service<
                http::Request<tonic::body::Body>,
            >>::Error: Into<StdError> + std::marker::Send + std::marker::Sync,
        {
            OperatorServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with the given encoding.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        /// Enable decompressing responses.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }
        ///
        pub async fn health(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/Health",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("screenroom.operator.v1.OperatorService", "Health"),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn discover_connection(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/DiscoverConnection",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "DiscoverConnection",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn save_connection(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/SaveConnection",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "SaveConnection",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn list_connections(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListConnections",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ListConnections",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn list_sources(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListSources",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ListSources",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn list_libraries(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListLibraries",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ListLibraries",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn list_runs(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListRuns",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("screenroom.operator.v1.OperatorService", "ListRuns"),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn list_candidates(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListCandidates",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ListCandidates",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn list_policies(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListPolicies",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ListPolicies",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn configure_policy(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ConfigurePolicy",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ConfigurePolicy",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_progress(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/GetProgress",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "GetProgress",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn start_run(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/StartRun",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("screenroom.operator.v1.OperatorService", "StartRun"),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_run_status(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/GetRunStatus",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "GetRunStatus",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn get_execution(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/GetExecution",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "GetExecution",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn pause_run(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/PauseRun",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("screenroom.operator.v1.OperatorService", "PauseRun"),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn resume_run(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ResumeRun",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ResumeRun",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        ///
        pub async fn cancel_run(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/CancelRun",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "CancelRun",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn list_jobs(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListJobs",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("screenroom.operator.v1.OperatorService", "ListJobs"),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn list_attempts(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListAttempts",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ListAttempts",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn list_redrives(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ListRedrives",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ListRedrives",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn redrive_job(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/RedriveJob",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "RedriveJob",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn observe_queue(
            &mut self,
            request: impl tonic::IntoRequest<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/screenroom.operator.v1.OperatorService/ObserveQueue",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "screenroom.operator.v1.OperatorService",
                        "ObserveQueue",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
    }
}
/// Generated server implementations.
pub mod operator_service_server {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use tonic::codegen::*;
    /// Generated trait containing gRPC methods that should be implemented for use with OperatorServiceServer.
    #[async_trait]
    pub trait OperatorService: std::marker::Send + std::marker::Sync + 'static {
        ///
        async fn health(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn discover_connection(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn save_connection(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn list_connections(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn list_sources(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn list_libraries(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn list_runs(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn list_candidates(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn list_policies(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn configure_policy(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn get_progress(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn start_run(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn get_run_status(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn get_execution(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn pause_run(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn resume_run(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        ///
        async fn cancel_run(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        async fn list_jobs(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        async fn list_attempts(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        async fn list_redrives(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        async fn redrive_job(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
        async fn observe_queue(
            &self,
            request: tonic::Request<super::OperatorRequest>,
        ) -> std::result::Result<
            tonic::Response<super::OperatorResponse>,
            tonic::Status,
        >;
    }
    /** Administrative operations for trusted deployments. Selectors do not authenticate callers.
*/
    #[derive(Debug)]
    pub struct OperatorServiceServer<T> {
        inner: Arc<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    impl<T> OperatorServiceServer<T> {
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }
        pub fn from_arc(inner: Arc<T>) -> Self {
            Self {
                inner,
                accept_compression_encodings: Default::default(),
                send_compression_encodings: Default::default(),
                max_decoding_message_size: None,
                max_encoding_message_size: None,
            }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> InterceptedService<Self, F>
        where
            F: tonic::service::Interceptor,
        {
            InterceptedService::new(Self::new(inner), interceptor)
        }
        /// Enable decompressing requests with the given encoding.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.accept_compression_encodings.enable(encoding);
            self
        }
        /// Compress responses with the given encoding, if the client supports it.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.send_compression_encodings.enable(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.max_decoding_message_size = Some(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.max_encoding_message_size = Some(limit);
            self
        }
    }
    impl<T, B> tonic::codegen::Service<http::Request<B>> for OperatorServiceServer<T>
    where
        T: OperatorService,
        B: Body + std::marker::Send + 'static,
        B::Error: Into<StdError> + std::marker::Send + 'static,
    {
        type Response = http::Response<tonic::body::Body>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            match req.uri().path() {
                "/screenroom.operator.v1.OperatorService/Health" => {
                    #[allow(non_camel_case_types)]
                    struct HealthSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for HealthSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::health(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = HealthSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/DiscoverConnection" => {
                    #[allow(non_camel_case_types)]
                    struct DiscoverConnectionSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for DiscoverConnectionSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::discover_connection(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = DiscoverConnectionSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/SaveConnection" => {
                    #[allow(non_camel_case_types)]
                    struct SaveConnectionSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for SaveConnectionSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::save_connection(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = SaveConnectionSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListConnections" => {
                    #[allow(non_camel_case_types)]
                    struct ListConnectionsSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListConnectionsSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_connections(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListConnectionsSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListSources" => {
                    #[allow(non_camel_case_types)]
                    struct ListSourcesSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListSourcesSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_sources(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListSourcesSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListLibraries" => {
                    #[allow(non_camel_case_types)]
                    struct ListLibrariesSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListLibrariesSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_libraries(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListLibrariesSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListRuns" => {
                    #[allow(non_camel_case_types)]
                    struct ListRunsSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListRunsSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_runs(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListRunsSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListCandidates" => {
                    #[allow(non_camel_case_types)]
                    struct ListCandidatesSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListCandidatesSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_candidates(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListCandidatesSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListPolicies" => {
                    #[allow(non_camel_case_types)]
                    struct ListPoliciesSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListPoliciesSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_policies(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListPoliciesSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ConfigurePolicy" => {
                    #[allow(non_camel_case_types)]
                    struct ConfigurePolicySvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ConfigurePolicySvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::configure_policy(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ConfigurePolicySvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/GetProgress" => {
                    #[allow(non_camel_case_types)]
                    struct GetProgressSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for GetProgressSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::get_progress(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetProgressSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/StartRun" => {
                    #[allow(non_camel_case_types)]
                    struct StartRunSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for StartRunSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::start_run(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = StartRunSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/GetRunStatus" => {
                    #[allow(non_camel_case_types)]
                    struct GetRunStatusSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for GetRunStatusSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::get_run_status(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetRunStatusSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/GetExecution" => {
                    #[allow(non_camel_case_types)]
                    struct GetExecutionSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for GetExecutionSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::get_execution(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetExecutionSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/PauseRun" => {
                    #[allow(non_camel_case_types)]
                    struct PauseRunSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for PauseRunSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::pause_run(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = PauseRunSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ResumeRun" => {
                    #[allow(non_camel_case_types)]
                    struct ResumeRunSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ResumeRunSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::resume_run(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ResumeRunSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/CancelRun" => {
                    #[allow(non_camel_case_types)]
                    struct CancelRunSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for CancelRunSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::cancel_run(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = CancelRunSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListJobs" => {
                    #[allow(non_camel_case_types)]
                    struct ListJobsSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListJobsSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_jobs(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListJobsSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListAttempts" => {
                    #[allow(non_camel_case_types)]
                    struct ListAttemptsSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListAttemptsSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_attempts(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListAttemptsSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ListRedrives" => {
                    #[allow(non_camel_case_types)]
                    struct ListRedrivesSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ListRedrivesSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::list_redrives(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ListRedrivesSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/RedriveJob" => {
                    #[allow(non_camel_case_types)]
                    struct RedriveJobSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for RedriveJobSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::redrive_job(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = RedriveJobSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/screenroom.operator.v1.OperatorService/ObserveQueue" => {
                    #[allow(non_camel_case_types)]
                    struct ObserveQueueSvc<T: OperatorService>(pub Arc<T>);
                    impl<
                        T: OperatorService,
                    > tonic::server::UnaryService<super::OperatorRequest>
                    for ObserveQueueSvc<T> {
                        type Response = super::OperatorResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::OperatorRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as OperatorService>::observe_queue(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ObserveQueueSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => {
                    Box::pin(async move {
                        let mut response = http::Response::new(
                            tonic::body::Body::default(),
                        );
                        let headers = response.headers_mut();
                        headers
                            .insert(
                                tonic::Status::GRPC_STATUS,
                                (tonic::Code::Unimplemented as i32).into(),
                            );
                        headers
                            .insert(
                                http::header::CONTENT_TYPE,
                                tonic::metadata::GRPC_CONTENT_TYPE,
                            );
                        Ok(response)
                    })
                }
            }
        }
    }
    impl<T> Clone for OperatorServiceServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self {
                inner,
                accept_compression_encodings: self.accept_compression_encodings,
                send_compression_encodings: self.send_compression_encodings,
                max_decoding_message_size: self.max_decoding_message_size,
                max_encoding_message_size: self.max_encoding_message_size,
            }
        }
    }
    /// Generated gRPC service name
    pub const SERVICE_NAME: &str = "screenroom.operator.v1.OperatorService";
    impl<T> tonic::server::NamedService for OperatorServiceServer<T> {
        const NAME: &'static str = SERVICE_NAME;
    }
}
