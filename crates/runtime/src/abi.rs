use synos_ipc::SharedBuffer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Operation {
    Yield = 1,
    ClockNow = 2,
    Wait = 3,
    Wake = 4,
    ThreadSpawn = 5,
    ThreadJoin = 6,
    ThreadExit = 7,
    MemoryMap = 8,
    MemoryUnmap = 9,
    IpcMap = 10,
    IpcNotify = 11,
    SynFsOpen = 12,
    SynFsClose = 13,
    SynFsRead = 14,
    SynFsWrite = 15,
    SynFsMetadata = 16,
    SynFsMkdir = 17,
    SynFsRmdir = 18,
    SynFsLink = 19,
    SynFsList = 20,
    SynFsLinks = 21,
    SynFsDelete = 22,
    RandomGet = 23,
    TerminalRead = 24,
    TerminalWrite = 25,
    ProcessSpawn = 26,
    ProcessExec = 27,
    ProcessWait = 28,
    ProcessCancel = 29,
    ProcessExit = 30,
    ProcessStatus = 31,
    MemoryProtect = 32,
    CapabilityQuery = 33,
    PipeCreate = 34,
    PipeRead = 35,
    PipeWrite = 36,
    PipeClose = 37,
    ThreadTlsGet = 38,
    ThreadTlsSet = 39,
    ArgumentsRead = 40,
    EnvironmentRead = 41,
}

impl Operation {
    pub const fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Yield),
            2 => Some(Self::ClockNow),
            3 => Some(Self::Wait),
            4 => Some(Self::Wake),
            5 => Some(Self::ThreadSpawn),
            6 => Some(Self::ThreadJoin),
            7 => Some(Self::ThreadExit),
            8 => Some(Self::MemoryMap),
            9 => Some(Self::MemoryUnmap),
            10 => Some(Self::IpcMap),
            11 => Some(Self::IpcNotify),
            12 => Some(Self::SynFsOpen),
            13 => Some(Self::SynFsClose),
            14 => Some(Self::SynFsRead),
            15 => Some(Self::SynFsWrite),
            16 => Some(Self::SynFsMetadata),
            17 => Some(Self::SynFsMkdir),
            18 => Some(Self::SynFsRmdir),
            19 => Some(Self::SynFsLink),
            20 => Some(Self::SynFsList),
            21 => Some(Self::SynFsLinks),
            22 => Some(Self::SynFsDelete),
            23 => Some(Self::RandomGet),
            24 => Some(Self::TerminalRead),
            25 => Some(Self::TerminalWrite),
            26 => Some(Self::ProcessSpawn),
            27 => Some(Self::ProcessExec),
            28 => Some(Self::ProcessWait),
            29 => Some(Self::ProcessCancel),
            30 => Some(Self::ProcessExit),
            31 => Some(Self::ProcessStatus),
            32 => Some(Self::MemoryProtect),
            33 => Some(Self::CapabilityQuery),
            34 => Some(Self::PipeCreate),
            35 => Some(Self::PipeRead),
            36 => Some(Self::PipeWrite),
            37 => Some(Self::PipeClose),
            38 => Some(Self::ThreadTlsGet),
            39 => Some(Self::ThreadTlsSet),
            40 => Some(Self::ArgumentsRead),
            41 => Some(Self::EnvironmentRead),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Request {
    pub operation: u16,
    pub flags: u16,
    pub reserved: u32,
    pub capability: u64,
    pub arguments: [u64; 6],
}

impl Request {
    pub const fn new(operation: Operation) -> Self {
        Self {
            operation: operation as u16,
            flags: 0,
            reserved: 0,
            capability: 0,
            arguments: [0; 6],
        }
    }

    pub const fn with_capability(mut self, capability: Capability) -> Self {
        self.capability = capability.raw();
        self
    }

    pub const fn with_buffer(mut self, buffer: SharedBuffer) -> Self {
        self.arguments[0] = buffer.region.raw() as u64;
        self.arguments[1] = buffer.offset as u64;
        self.arguments[2] = buffer.length as u64;
        self.arguments[3] = buffer.writable as u64;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Response {
    pub status: u32,
    pub flags: u32,
    pub values: [u64; 4],
}

impl Response {
    pub const EMPTY: Self = Self {
        status: 0,
        flags: 0,
        values: [0; 4],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Capability(u64);

impl Capability {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

pub trait SystemCall {
    fn call(&self, request: Request) -> Response;
}

pub type GateFn = unsafe extern "C" fn(*const Request, *mut Response);

#[derive(Clone, Copy)]
pub struct NativeGate {
    gate: GateFn,
}

impl NativeGate {
    /// Creates a call gate supplied by the SynOS process loader.
    ///
    /// # Safety
    ///
    /// `gate` must obey the SynOS system-call ABI for the life of this value.
    pub const unsafe fn new(gate: GateFn) -> Self {
        Self { gate }
    }
}

impl SystemCall for NativeGate {
    fn call(&self, request: Request) -> Response {
        let mut response = Response::EMPTY;
        unsafe { (self.gate)(&request, &mut response) }
        response
    }
}
