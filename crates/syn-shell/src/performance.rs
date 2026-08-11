use synos_status::Status;
use synos_system_model::{
    command::StructuredOutput,
    performance::{PerformanceBudget, PerformanceDiagnostics, TailLatencyWindow},
};
use synos_time_sync::MonotonicClock;

use crate::{
    Error,
    interpreter::{CommandExecutor, ExecutionToken},
    parser::{CommandCall, Value},
};

const MAX_ROUTE_METRICS: usize = 64;

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopCommandClock;

impl MonotonicClock for NoopCommandClock {
    fn now_us(&self) -> u64 {
        0
    }
}

#[derive(Clone, Copy)]
struct Measurement {
    generation: u32,
    inner: Option<ExecutionToken>,
    command: Option<CommandCall>,
    started_at_us: u64,
    request_bytes: u32,
}

impl Measurement {
    const EMPTY: Self = Self {
        generation: 0,
        inner: None,
        command: None,
        started_at_us: 0,
        request_bytes: 0,
    };
}

/// Adds bounded performance diagnostics to any public shell command executor.
/// The wrapper records counters only; command arguments and output values never
/// enter the diagnostics.
pub struct BudgetedCommandExecutor<E, C, const CAPACITY: usize> {
    inner: E,
    clock: C,
    slots: [Measurement; CAPACITY],
    tails: [TailLatencyWindow<32>; MAX_ROUTE_METRICS],
    last_diagnostics: PerformanceDiagnostics,
}

impl<E, C: MonotonicClock, const CAPACITY: usize> BudgetedCommandExecutor<E, C, CAPACITY> {
    pub const fn new(inner: E, clock: C) -> Self {
        Self {
            inner,
            clock,
            slots: [Measurement::EMPTY; CAPACITY],
            tails: [const { TailLatencyWindow::new() }; MAX_ROUTE_METRICS],
            last_diagnostics: PerformanceDiagnostics::empty(),
        }
    }

    pub fn inner(&self) -> &E {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut E {
        &mut self.inner
    }

    pub const fn last_diagnostics(&self) -> PerformanceDiagnostics {
        self.last_diagnostics
    }

    fn finish(
        &mut self,
        slot: usize,
        command: CommandCall,
        completion: Result<StructuredOutput, Status>,
    ) -> Option<Result<StructuredOutput, Status>> {
        let measurement = self.slots[slot];
        self.slots[slot].inner = None;
        self.slots[slot].command = None;
        let service_time_us = self.clock.now_us().saturating_sub(measurement.started_at_us);
        let route = command.route.raw() as usize;
        let route_index = route.min(MAX_ROUTE_METRICS - 1);
        self.tails[route_index].record(service_time_us);
        let budget = PerformanceBudget::for_route(command.route.raw());
        let response_bytes = completion
            .as_ref()
            .map(|output| output.encoded_bytes())
            .unwrap_or(0);
        let diagnostics = PerformanceDiagnostics::new(
            0,
            service_time_us,
            0,
            measurement.request_bytes,
            response_bytes,
            self.tails[route_index].p99(),
            budget,
        );
        self.last_diagnostics = diagnostics;
        match completion {
            Ok(mut output) => {
                if output.insert_performance_diagnostics(diagnostics).is_err() {
                    Some(Err(Status::NO_SPACE))
                } else {
                    Some(Ok(output))
                }
            }
            Err(status) => Some(Err(status)),
        }
    }
}

impl<E: CommandExecutor, C: MonotonicClock, const CAPACITY: usize> CommandExecutor
    for BudgetedCommandExecutor<E, C, CAPACITY>
{
    fn submit(
        &mut self,
        command: CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        let slot = self
            .slots
            .iter()
            .position(|measurement| measurement.inner.is_none())
            .ok_or(Error::Capacity)?;
        let inner = self.inner.submit(command, pipeline_input)?;
        let generation = self.slots[slot].generation.wrapping_add(1).max(1);
        self.slots[slot] = Measurement {
            generation,
            inner: Some(inner),
            command: Some(command),
            started_at_us: self.clock.now_us(),
            request_bytes: request_bytes(command),
        };
        ExecutionToken::new((generation as u64) << 32 | slot as u64).ok_or(Error::InvalidHandle)
    }

    fn poll(
        &mut self,
        token: ExecutionToken,
    ) -> Option<Result<StructuredOutput, Status>> {
        let slot = token.raw() as u32 as usize;
        let generation = (token.raw() >> 32) as u32;
        let measurement = *self.slots.get(slot)?;
        if measurement.generation != generation {
            return Some(Err(Status::NOT_FOUND))
        }
        let inner = measurement.inner?;
        let completion = self.inner.poll(inner)?;
        self.finish(slot, measurement.command?, completion)
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        let slot = token.raw() as u32 as usize;
        let generation = (token.raw() >> 32) as u32;
        let measurement = *self.slots.get(slot).ok_or(Error::InvalidHandle)?;
        if measurement.generation != generation {
            return Err(Error::InvalidHandle)
        }
        let inner = measurement.inner.ok_or(Error::InvalidHandle)?;
        self.inner.cancel(inner)?;
        self.slots[slot].inner = None;
        self.slots[slot].command = None;
        Ok(())
    }
}

fn request_bytes(command: CommandCall) -> u32 {
    let mut bytes = command.command.as_str().len();
    for argument in command.arguments() {
        bytes += argument.name.as_str().len();
        bytes += match argument.value {
            Value::Boolean(_) => 1,
            Value::Integer(_) => 8,
            Value::Text(value) => value.len(),
        };
    }
    bytes as u32
}
