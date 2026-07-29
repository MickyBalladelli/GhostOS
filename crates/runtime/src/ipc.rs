use synos_ipc::SharedRegionId;

use crate::{Capability, Error, Operation, Request, Runtime, SystemCall};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IpcAccess {
    Send = 1,
    Receive = 2,
    SendAndReceive = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcMapping {
    pub endpoint: Capability,
    pub memory: Capability,
    pub region: SharedRegionId,
    pub address: u64,
    pub length: u64,
    pub access: IpcAccess,
}

impl<S: SystemCall> Runtime<S> {
    pub fn map_ipc(
        &self,
        endpoint: Capability,
        memory: Capability,
        region: SharedRegionId,
        length: u64,
        access: IpcAccess,
    ) -> Result<IpcMapping, Error> {
        let mut request = Request::new(Operation::IpcMap).with_capability(endpoint);
        request.arguments[0] = memory.raw();
        request.arguments[1] = region.raw() as u64;
        request.arguments[2] = length;
        request.arguments[3] = access as u64;
        let response = self.execute(request)?;
        Ok(IpcMapping {
            endpoint,
            memory,
            region,
            address: response.values[0],
            length,
            access,
        })
    }

    pub fn notify_ipc(&self, mapping: IpcMapping) -> Result<(), Error> {
        let mut request = Request::new(Operation::IpcNotify).with_capability(mapping.endpoint);
        request.arguments[0] = mapping.region.raw() as u64;
        request.arguments[1] = mapping.access as u64;
        self.execute(request)?;
        Ok(())
    }
}
