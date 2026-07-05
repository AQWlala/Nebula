#[cfg(feature = "wasm-sandbox")]
mod wasm_sandbox_tests {
    use nebula::skills::{Capability, CapabilitySet, WasmSandbox, WasmSandboxConfig};

    #[test]
    fn wasm_sandbox_config_default_is_llm_only() {
        let config = WasmSandboxConfig::default();
        assert!(config.capabilities.has(Capability::LlmCall));
        assert!(!config.capabilities.has(Capability::FileRead));
        assert!(!config.capabilities.has(Capability::FileWrite));
        assert!(!config.capabilities.has(Capability::Network));
        assert_eq!(config.max_fuel, 1_000_000);
    }

    #[test]
    fn wasm_sandbox_new_with_llm_only_succeeds() {
        let config = WasmSandboxConfig::default();
        let sandbox = WasmSandbox::new(&config);
        assert!(sandbox.is_ok());
    }

    #[test]
    fn wasm_sandbox_new_with_full_trust_succeeds() {
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::FileRead);
        caps.grant(Capability::FileWrite);
        caps.grant(Capability::Network);
        caps.grant(Capability::LlmCall);
        let config = WasmSandboxConfig {
            capabilities: caps,
            max_fuel: 1_000_000,
        };
        let sandbox = WasmSandbox::new(&config);
        assert!(sandbox.is_ok());
    }

    #[test]
    fn wasm_sandbox_capabilities_returns_configured_set() {
        let config = WasmSandboxConfig::default();
        let sandbox = WasmSandbox::new(&config).unwrap();
        assert!(sandbox.capabilities().has(Capability::LlmCall));
    }
}
