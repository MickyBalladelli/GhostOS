#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStats {
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub drops: u64,
    pub errors: u64,
    pub rx_queue_depth: usize,
    pub tx_queue_depth: usize,
    pub rx_queue_peak: usize,
    pub tx_queue_peak: usize,
    pub arp_failures: u64,
    pub dhcp_retries: u64,
    pub icmp_rx: u64,
    pub icmp_tx: u64,
    pub icmp_loss: u64,
}

impl NetworkStats {
    pub const fn new() -> Self {
        Self {
            rx_packets: 0,
            rx_bytes: 0,
            tx_packets: 0,
            tx_bytes: 0,
            drops: 0,
            errors: 0,
            rx_queue_depth: 0,
            tx_queue_depth: 0,
            rx_queue_peak: 0,
            tx_queue_peak: 0,
            arp_failures: 0,
            dhcp_retries: 0,
            icmp_rx: 0,
            icmp_tx: 0,
            icmp_loss: 0,
        }
    }

    pub const fn snapshot(self) -> Self {
        self
    }

    pub fn record_rx(&mut self, bytes: usize) {
        self.rx_packets = self.rx_packets.saturating_add(1);
        self.rx_bytes = self.rx_bytes.saturating_add(bytes as u64);
    }

    pub fn record_tx(&mut self, bytes: usize) {
        self.tx_packets = self.tx_packets.saturating_add(1);
        self.tx_bytes = self.tx_bytes.saturating_add(bytes as u64);
    }

    pub fn record_drop(&mut self) {
        self.drops = self.drops.saturating_add(1);
    }

    pub fn record_error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }

    pub fn record_arp_failure(&mut self) {
        self.arp_failures = self.arp_failures.saturating_add(1);
    }

    pub fn record_dhcp_retry(&mut self) {
        self.dhcp_retries = self.dhcp_retries.saturating_add(1);
    }

    pub fn record_icmp_rx(&mut self) {
        self.icmp_rx = self.icmp_rx.saturating_add(1);
    }

    pub fn record_icmp_tx(&mut self) {
        self.icmp_tx = self.icmp_tx.saturating_add(1);
    }

    pub fn record_icmp_loss(&mut self) {
        self.icmp_loss = self.icmp_loss.saturating_add(1);
    }

    pub fn set_queue_depth(&mut self, rx: usize, tx: usize) {
        self.rx_queue_depth = rx;
        self.tx_queue_depth = tx;
        self.rx_queue_peak = self.rx_queue_peak.max(rx);
        self.tx_queue_peak = self.tx_queue_peak.max(tx);
    }
}

impl Default for NetworkStats {
    fn default() -> Self {
        Self::new()
    }
}
