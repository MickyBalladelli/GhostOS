use core::fmt::{self, Write};

use crate::{
    CapabilityKind, CapabilitySample, NodeHealth, NodeSample, TopologySnapshot,
};

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_HEADER: &str = "\x1b[1;96m";
const ANSI_LABEL: &str = "\x1b[1;37m";
const ANSI_DIM: &str = "\x1b[90m";
const ANSI_GOOD: &str = "\x1b[92m";
const ANSI_WARN: &str = "\x1b[93m";
const ANSI_BAD: &str = "\x1b[91m";
const ANSI_RAM: &str = "\x1b[94m";
const ANSI_VRAM: &str = "\x1b[95m";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Viewport {
    pub columns: u16,
    pub rows: u16,
}

impl Viewport {
    pub const MIN_COLUMNS: u16 = 64;
    pub const MIN_ROWS: u16 = 20;

    pub const fn new(columns: u16, rows: u16) -> Option<Self> {
        if columns < Self::MIN_COLUMNS || rows < Self::MIN_ROWS {
            None
        } else {
            Some(Self { columns, rows })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    Output,
    ViewportTooSmall,
}

/// ANSI dashboard renderer.
///
/// SynOS' serial, VGA, and GOP framebuffer consoles all consume this stream.
/// Cursor-home redraws avoid scrolling and hide the cursor while the monitor is
/// active.
pub struct DashboardRenderer {
    viewport: Viewport,
    first_frame: bool,
}

impl DashboardRenderer {
    pub const fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            first_frame: true,
        }
    }

    pub fn enter<W: Write>(&mut self, output: &mut W) -> Result<(), RenderError> {
        output
            .write_str("\x1b[?25l\x1b[2J\x1b[H")
            .map_err(|_| RenderError::Output)?;
        self.first_frame = false;
        Ok(())
    }

    pub fn leave<W: Write>(&mut self, output: &mut W) -> Result<(), RenderError> {
        output
            .write_str("\x1b[0m\x1b[?25h\n")
            .map_err(|_| RenderError::Output)
    }

    pub fn render<
        W: Write,
        const NODES: usize,
        const CAPABILITIES: usize,
    >(
        &mut self,
        snapshot: &TopologySnapshot<NODES, CAPABILITIES>,
        output: &mut W,
    ) -> Result<(), RenderError> {
        if self.viewport.columns < Viewport::MIN_COLUMNS
            || self.viewport.rows < Viewport::MIN_ROWS
        {
            return Err(RenderError::ViewportTooSmall)
        }
        if self.first_frame {
            self.enter(output)?
        } else {
            output
                .write_str("\x1b[H")
                .map_err(|_| RenderError::Output)?
        }

        write!(
            output,
            "{ANSI_HEADER}SYNOS-TOP{ANSI_RESET}  generation={}  sample={}us",
            snapshot.generation(),
            snapshot.sampled_at_us(),
        )
        .map_err(|_| RenderError::Output)?;
        erase_line(output)?;

        section(output, "CLUSTER RAM / VRAM HEATMAP")?;
        let node_rows = ((self.viewport.rows as usize).saturating_sub(12) / 2)
            .clamp(2, 8);
        for sample in snapshot.nodes().take(node_rows) {
            render_memory(sample, output, self.viewport.columns as usize)?
        }
        if snapshot.nodes().next().is_none() {
            write!(output, "{ANSI_DIM}waiting for node telemetry{ANSI_RESET}")
                .map_err(|_| RenderError::Output)?;
            erase_line(output)?
        }

        section(output, "REMOTE DSM PAGE-FAULT LATENCY")?;
        let latency_rows = node_rows.min(4);
        let latency_max = snapshot
            .nodes()
            .map(|node| node.dsm_latency_p99_ns)
            .max()
            .unwrap_or(1)
            .max(1);
        for sample in snapshot.nodes().take(latency_rows) {
            render_latency(sample, latency_max, output)?
        }
        if snapshot.nodes().next().is_none() {
            write!(output, "{ANSI_DIM}no remote faults sampled{ANSI_RESET}")
                .map_err(|_| RenderError::Output)?;
            erase_line(output)?
        }

        section(output, "DYNAMIC CAPABILITY GRAPH")?;
        let used_rows = 5 + node_rows * 2 + latency_rows;
        let graph_rows = (self.viewport.rows as usize)
            .saturating_sub(used_rows)
            .max(1);
        let rendered = render_capability_graph(snapshot, graph_rows, output)?;
        if rendered == 0 {
            write!(output, "{ANSI_DIM}no visible capabilities{ANSI_RESET}")
                .map_err(|_| RenderError::Output)?;
            erase_line(output)?
        }
        output
            .write_str("\x1b[0m\x1b[J")
            .map_err(|_| RenderError::Output)
    }
}

fn section<W: Write>(output: &mut W, title: &str) -> Result<(), RenderError> {
    write!(output, "{ANSI_LABEL}{title}{ANSI_RESET}")
        .map_err(|_| RenderError::Output)?;
    erase_line(output)
}

fn erase_line<W: Write>(output: &mut W) -> Result<(), RenderError> {
    output
        .write_str("\x1b[K\r\n")
        .map_err(|_| RenderError::Output)
}

fn render_memory<W: Write>(
    sample: &NodeSample,
    output: &mut W,
    columns: usize,
) -> Result<(), RenderError> {
    let health = match sample.health {
        NodeHealth::Healthy => (ANSI_GOOD, "UP"),
        NodeHealth::Degraded => (ANSI_WARN, "WARN"),
        NodeHealth::Failed => (ANSI_BAD, "DOWN"),
    };
    write!(
        output,
        "{color}N{node:02} {state:<4}{ANSI_RESET} RAM ",
        color = health.0,
        node = sample.node.raw(),
        state = health.1,
    )
    .map_err(|_| RenderError::Output)?;
    usage_bar(
        output,
        sample.ram_used_bytes,
        sample.ram_total_bytes,
        columns.saturating_sub(37).clamp(8, 32),
        ANSI_RAM,
    )?;
    write!(
        output,
        " {:>3}% {}/{} MiB",
        percent(sample.ram_used_bytes, sample.ram_total_bytes),
        mib(sample.ram_used_bytes),
        mib(sample.ram_total_bytes),
    )
    .map_err(|_| RenderError::Output)?;
    erase_line(output)?;

    write!(output, "           VRAM ")
        .map_err(|_| RenderError::Output)?;
    usage_bar(
        output,
        sample.vram_used_bytes,
        sample.vram_total_bytes,
        columns.saturating_sub(37).clamp(8, 32),
        ANSI_VRAM,
    )?;
    write!(
        output,
        " {:>3}% {}/{} MiB",
        percent(sample.vram_used_bytes, sample.vram_total_bytes),
        mib(sample.vram_used_bytes),
        mib(sample.vram_total_bytes),
    )
    .map_err(|_| RenderError::Output)?;
    erase_line(output)
}

fn render_latency<W: Write>(
    sample: &NodeSample,
    max: u32,
    output: &mut W,
) -> Result<(), RenderError> {
    let color = if sample.dsm_latency_p99_ns <= 50_000 {
        ANSI_GOOD
    } else if sample.dsm_latency_p99_ns <= 250_000 {
        ANSI_WARN
    } else {
        ANSI_BAD
    };
    let filled = scale(sample.dsm_latency_p99_ns as u64, max as u64, 20);
    write!(
        output,
        "N{:02} {color}",
        sample.node.raw(),
    )
    .map_err(|_| RenderError::Output)?;
    blocks(output, filled, 20)?;
    write!(
        output,
        "{ANSI_RESET} p50={:>7}ns p99={:>7}ns faults={}",
        sample.dsm_latency_p50_ns,
        sample.dsm_latency_p99_ns,
        sample.remote_faults,
    )
    .map_err(|_| RenderError::Output)?;
    erase_line(output)
}

fn render_capability_graph<
    W: Write,
    const NODES: usize,
    const CAPABILITIES: usize,
>(
    snapshot: &TopologySnapshot<NODES, CAPABILITIES>,
    max_rows: usize,
    output: &mut W,
) -> Result<usize, RenderError> {
    let mut rendered = 0;
    for root in snapshot
        .capabilities()
        .filter(|capability| capability.parent.is_none())
    {
        render_capability_branch(
            root,
            snapshot,
            0,
            max_rows,
            &mut rendered,
            output,
        )?
    }
    Ok(rendered)
}

fn render_capability_branch<
    W: Write,
    const NODES: usize,
    const CAPABILITIES: usize,
>(
    capability: &CapabilitySample,
    snapshot: &TopologySnapshot<NODES, CAPABILITIES>,
    depth: usize,
    max_rows: usize,
    rendered: &mut usize,
    output: &mut W,
) -> Result<(), RenderError> {
    if *rendered >= max_rows {
        return Ok(())
    }
    for _ in 0..depth {
        output
            .write_str("  ")
            .map_err(|_| RenderError::Output)?
    }
    let branch = if capability.parent.is_some() { "+-" } else { "*" };
    write!(
        output,
        "{ANSI_DIM}{branch}{ANSI_RESET} {ANSI_LABEL}{:016x}{ANSI_RESET} {:<5} owner={:<3} rights={:03x}",
        capability.handle,
        capability_kind(capability.kind),
        capability.owner,
        capability.rights,
    )
    .map_err(|_| RenderError::Output)?;
    erase_line(output)?;
    *rendered += 1;
    for child in snapshot
        .capabilities()
        .filter(|candidate| candidate.parent == Some(capability.handle))
    {
        render_capability_branch(
            child,
            snapshot,
            (depth + 1).min(6),
            max_rows,
            rendered,
            output,
        )?
    }
    Ok(())
}

const fn capability_kind(kind: CapabilityKind) -> &'static str {
    match kind {
        CapabilityKind::Memory => "MEM",
        CapabilityKind::AddressSpace => "AS",
        CapabilityKind::Ipc => "IPC",
        CapabilityKind::Lock => "LOCK",
        CapabilityKind::Namespace => "NAME",
        CapabilityKind::Other => "OTHER",
    }
}

fn usage_bar<W: Write>(
    output: &mut W,
    used: u64,
    total: u64,
    width: usize,
    color: &str,
) -> Result<(), RenderError> {
    let filled = scale(used, total, width);
    output.write_str(color).map_err(|_| RenderError::Output)?;
    blocks(output, filled, width)?;
    output
        .write_str(ANSI_RESET)
        .map_err(|_| RenderError::Output)
}

fn blocks<W: Write>(
    output: &mut W,
    filled: usize,
    width: usize,
) -> Result<(), RenderError> {
    output.write_char('[').map_err(|_| RenderError::Output)?;
    for index in 0..width {
        output
            .write_char(if index < filled { '#' } else { '.' })
            .map_err(|_| RenderError::Output)?
    }
    output.write_char(']').map_err(|_| RenderError::Output)
}

const fn scale(value: u64, total: u64, width: usize) -> usize {
    if total == 0 {
        0
    } else {
        let bounded = if value < total { value } else { total };
        ((bounded as u128 * width as u128) / total as u128) as usize
    }
}

const fn percent(used: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        let bounded = if used < total { used } else { total };
        ((bounded as u128 * 100) / total as u128) as u64
    }
}

const fn mib(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

impl From<fmt::Error> for RenderError {
    fn from(_: fmt::Error) -> Self {
        Self::Output
    }
}
