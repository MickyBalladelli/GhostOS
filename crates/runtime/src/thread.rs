use crate::{Capability, Error, Operation, Request, Runtime, SystemCall};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Thread {
    capability: Capability,
}

impl Thread {
    pub const fn capability(self) -> Capability {
        self.capability
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadStart {
    pub image: Capability,
    pub entry_offset: u64,
    pub stack: Capability,
    pub stack_top_offset: u64,
    pub argument: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitWord {
    pub region: Capability,
    pub offset: u32,
}

impl<S: SystemCall> Runtime<S> {
    pub fn yield_now(&self) -> Result<(), Error> {
        self.execute(Request::new(Operation::Yield))?;
        Ok(())
    }

    pub fn spawn(&self, start: ThreadStart) -> Result<Thread, Error> {
        let mut request = Request::new(Operation::ThreadSpawn).with_capability(start.image);
        request.arguments = [
            start.entry_offset,
            start.stack.raw(),
            start.stack_top_offset,
            start.argument,
            0,
            0,
        ];
        let response = self.execute(request)?;
        let capability = Capability::from_raw(response.values[0]).ok_or(Error::InvalidResponse)?;
        Ok(Thread { capability })
    }

    pub fn join(&self, thread: Thread) -> Result<u64, Error> {
        let response =
            self.execute(Request::new(Operation::ThreadJoin).with_capability(thread.capability))?;
        Ok(response.values[0])
    }

    pub fn thread_exit(&self, status: i32) -> Result<(), Error> {
        let mut request = Request::new(Operation::ThreadExit);
        request.arguments[0] = status as i64 as u64;
        self.execute(request)?;
        Ok(())
    }

    pub fn wait(
        &self,
        word: WaitWord,
        expected: u32,
        timeout_nanoseconds: Option<u64>,
    ) -> Result<bool, Error> {
        let mut request = Request::new(Operation::Wait).with_capability(word.region);
        request.arguments[0] = word.offset as u64;
        request.arguments[1] = expected as u64;
        request.arguments[2] = timeout_nanoseconds.unwrap_or(u64::MAX);
        let response = self.execute(request)?;
        Ok(response.values[0] != 0)
    }

    pub fn wake(&self, word: WaitWord, count: u32) -> Result<u32, Error> {
        let mut request = Request::new(Operation::Wake).with_capability(word.region);
        request.arguments[0] = word.offset as u64;
        request.arguments[1] = count as u64;
        let response = self.execute(request)?;
        u32::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)
    }
}
