use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RuntimeCapability {
    #[serde(rename = "provider.runtime.v1")]
    ProviderRuntimeV1,
    #[serde(rename = "provider.gateway.v1")]
    ProviderGatewayV1,
    /// The Provider consumes a worker-private `COWBOY_EXECUTION_DESCRIPTOR` and
    /// binds every native turn to that endpoint. No local execution fallback.
    #[serde(rename = "provider.execution-jsonrpc.v1")]
    ProviderExecutionJsonrpcV1,
    #[serde(rename = "provider.mcp-bindings.v1")]
    ProviderMcpBindingsV1,
    #[serde(rename = "provider.managed-profiles.v1")]
    ProviderManagedProfilesV1,
    #[serde(rename = "provider.managed-image.v1")]
    ProviderManagedImageV1,
}
