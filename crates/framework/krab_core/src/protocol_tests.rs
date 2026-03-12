use std::collections::HashMap;

use axum::body::Body;
use axum::http::Request;

use crate::http::{route_family_protocol, runtime_switch_header_rejected_by_default};
use crate::protocol::{
    DeploymentTopology, ExposureMode, ProtocolConfig, ProtocolKind, ProtocolPolicy,
    ServiceCapabilities,
};

#[test]
fn test_topology_parse_single_service() {
    assert_eq!(
        DeploymentTopology::parse("single_service"),
        Some(DeploymentTopology::SingleService)
    );
}

#[test]
fn test_topology_parse_split_services() {
    assert_eq!(
        DeploymentTopology::parse("split_services"),
        Some(DeploymentTopology::SplitServices)
    );
}

#[test]
fn test_config_validation_default_in_enabled() {
    let config = ProtocolConfig {
        exposure_mode: ExposureMode::Multi,
        enabled_protocols: vec![ProtocolKind::Rest],
        default_protocol: ProtocolKind::Graphql,
        topology: DeploymentTopology::SingleService,
        policy: ProtocolPolicy::default(),
        allow_runtime_switch_header: false,
    };

    let errors = config.validate().expect_err("validation should fail");
    assert!(errors.iter().any(|e| e.contains("default protocol")));
}

#[test]
fn test_config_validation_single_mode_one_protocol() {
    let config = ProtocolConfig {
        exposure_mode: ExposureMode::Single,
        enabled_protocols: vec![ProtocolKind::Rest, ProtocolKind::Graphql],
        default_protocol: ProtocolKind::Rest,
        topology: DeploymentTopology::SingleService,
        policy: ProtocolPolicy::default(),
        allow_runtime_switch_header: false,
    };

    let errors = config.validate().expect_err("validation should fail");
    assert!(errors.iter().any(|e| e.contains("single exposure mode")));
}

#[test]
fn test_config_validation_tenant_override_protocol_must_be_enabled() {
    let mut tenant_overrides = HashMap::new();
    tenant_overrides.insert("tenant-a".to_string(), vec![ProtocolKind::Rpc]);

    let config = ProtocolConfig {
        exposure_mode: ExposureMode::Single,
        enabled_protocols: vec![ProtocolKind::Rest],
        default_protocol: ProtocolKind::Rest,
        topology: DeploymentTopology::SingleService,
        policy: ProtocolPolicy {
            restricted_operations: HashMap::new(),
            tenant_overrides,
        },
        allow_runtime_switch_header: false,
    };

    let errors = config.validate().expect_err("validation should fail");
    assert!(errors
        .iter()
        .any(|e| e.contains("tenant override") && e.contains("unsupported protocol")));
}

#[test]
fn test_parse_protocol_kind_case_insensitive() {
    assert_eq!(ProtocolKind::parse("REST"), Some(ProtocolKind::Rest));
    assert_eq!(ProtocolKind::parse("Graphql"), Some(ProtocolKind::Graphql));
    assert_eq!(ProtocolKind::parse("rpc"), Some(ProtocolKind::Rpc));
    assert_eq!(ProtocolKind::parse("GRPC"), Some(ProtocolKind::Rpc));
}

#[test]
fn test_parse_protocol_kind_invalid() {
    assert_eq!(ProtocolKind::parse("soap"), None);
    assert_eq!(ProtocolKind::parse(""), None);
    assert_eq!(ProtocolKind::parse("xml"), None);
}

#[test]
fn test_route_family_resolves_protocol_rest() {
    assert_eq!(
        route_family_protocol("/api/v1/users/me"),
        Some(ProtocolKind::Rest)
    );
}

#[test]
fn test_route_family_resolves_protocol_graphql() {
    assert_eq!(
        route_family_protocol("/api/v1/graphql"),
        Some(ProtocolKind::Graphql)
    );
}

#[test]
fn test_route_family_resolves_protocol_rpc() {
    assert_eq!(
        route_family_protocol("/api/v1/rpc"),
        Some(ProtocolKind::Rpc)
    );
}

#[test]
fn test_runtime_switch_header_rejected_by_default() {
    let req = Request::builder()
        .uri("/api/v1/users/me")
        .header("x-krab-protocol", "graphql")
        .body(Body::empty())
        .expect("request should build");

    let config = ProtocolConfig {
        exposure_mode: ExposureMode::Single,
        enabled_protocols: vec![ProtocolKind::Rest],
        default_protocol: ProtocolKind::Rest,
        topology: DeploymentTopology::SingleService,
        policy: ProtocolPolicy::default(),
        allow_runtime_switch_header: false,
    };

    assert!(runtime_switch_header_rejected_by_default(&req, &config));
}

#[test]
fn test_capabilities_struct_shape_is_constructible() {
    let mut routes = HashMap::new();
    routes.insert(ProtocolKind::Rest, "/api/v1/users".to_string());
    let caps = ServiceCapabilities {
        service: "users".to_string(),
        default_protocol: ProtocolKind::Rest,
        supported_protocols: vec![ProtocolKind::Rest],
        protocol_routes: routes,
    };

    assert_eq!(caps.service, "users");
    assert_eq!(caps.default_protocol, ProtocolKind::Rest);
    assert_eq!(caps.supported_protocols, vec![ProtocolKind::Rest]);
    assert_eq!(
        caps.protocol_routes.get(&ProtocolKind::Rest),
        Some(&"/api/v1/users".to_string())
    );
}
