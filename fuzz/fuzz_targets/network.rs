#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_shell::network::register_network_commands;
use ghostos_shell::parser::CommandRegistry;
use ghostos_netd::{
    inspect_frame, DhcpClient, DhcpError, DhcpLease, DhcpLeaseRecord, DhcpLeaseRuntime,
    InterfaceConfig, NetworkFrameKind, StaticSnapshot,
};

struct NoopRuntime;

impl DhcpLeaseRuntime for NoopRuntime {
    fn apply_lease(&mut self, _interface: &str, _lease: &DhcpLease) -> Result<(), DhcpError> {
        Ok(())
    }

    fn restore_static(
        &mut self,
        _interface: &str,
        _snapshot: &StaticSnapshot,
    ) -> Result<(), DhcpError> {
        Ok(())
    }
}

fn fuzz_command(data: &[u8]) {
    let mut registry = CommandRegistry::<16>::new();
    let _ = register_network_commands(&mut registry);
    let text = String::from_utf8_lossy(data);
    let command = if data.first().copied().unwrap_or(0) & 1 == 0 {
        format!("PING {text} /COUNT=1 /TIMEOUT=1 /SIZE=0 /IPV4")
    } else {
        format!("RESOLVE {text} /TIMEOUT=1 /IPV4")
    };
    let _ = registry.parse(&command);
}

fuzz_target!(|data: &[u8]| {
    let bounded = &data[..data.len().min(4096)];
    let _ = inspect_frame(bounded);
    let _ = DhcpLeaseRecord::decode(bounded);
    let _ = DhcpClient::new("eth0", [0x02, 0, 0, 0, 0, 1], 0x534E_4F53).map(|mut client| {
        let _ = client.authorize(0xFF);
        let _ = client.start(0);
        let mut runtime = NoopRuntime;
        let _ = client.handle_packet(bounded, 1, &mut runtime);
        client
    });
    let _ = InterfaceConfig::new([10, 5, 0, 2], [255, 255, 255, 0]);
    let _ = NetworkFrameKind::Invalid;
    fuzz_command(bounded);
});
