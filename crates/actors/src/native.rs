use crate::{ActorEndpoint, ActorError, ActorId, ActorSystem, DsmMailbox, DirectoryEntry, NodeRoute};
use ghostos_fabric::NodeId;
#[repr(C)]
#[derive(Clone, Copy)]
struct Entry { local: u64, node: u32, endpoint: u32, generation: u32, kind: u8 }
#[repr(C)]
#[derive(Clone, Copy)]
struct Route {
    range_start: u64, range_length: u64, auth_start: u64, auth_length: u64, expires_at_us: u64,
    node: u32, subject: u32, lease_epoch: u32, write: bool, occupied: bool,
}
fn failure<E>(code: i32) -> ActorError<E> {
    match code {
        1 => ActorError::AlreadyRegistered, 2 => ActorError::Capacity,
        3 => ActorError::CorruptEnvelope, 4 => ActorError::InvalidActor,
        5 => ActorError::InvalidEndpoint, 6 => ActorError::InvalidMailbox,
        7 => ActorError::NodeRouteMissing, 8 => ActorError::NotFound,
        9 => panic!("attempt to add with overflow"),
        _ => unreachable!("native actor result"),
    }
}
fn result<E>(code: i32) -> Result<(), ActorError<E>> {
    if code == 0 { Ok(()) } else { Err(failure(code)) }
}
fn kind(endpoint: ActorEndpoint) -> (u8, u32) {
    match endpoint {
        ActorEndpoint::Local(channel) => (1, channel.raw()),
        ActorEndpoint::Remote(mailbox) => (2, mailbox.destination.raw()),
    }
}
fn entries<const N: usize>(directory: &[Option<DirectoryEntry>; N]) -> [Entry; N] {
    directory.map(|entry| match entry {
        Some(entry) => {
            let (kind, endpoint) = kind(entry.endpoint);
            Entry { local: entry.actor.local, node: entry.actor.node.raw(), endpoint,
                generation: entry.generation, kind }
        }
        None => Entry { local: 0, node: 0, endpoint: 0, generation: 0, kind: 0 },
    })
}
fn route_view(route: DsmMailbox, occupied: bool) -> Route {
    Route { range_start: route.range.start, range_length: route.range.length,
        auth_start: route.authority.range.start, auth_length: route.authority.range.length,
        expires_at_us: route.authority.expires_at_us, node: route.destination.raw(),
        subject: route.authority.subject.raw(), lease_epoch: route.authority.lease_epoch,
        write: route.authority.write, occupied }
}
fn routes<const N: usize>(routes: &[Option<NodeRoute>; N]) -> [Route; N] {
    routes.map(|route| match route {
        Some(route) => {
            let mut view = route_view(route.supervisor, true);
            view.node = route.node.raw();
            view
        }
        None => Route { range_start: 0, range_length: 0, auth_start: 0, auth_length: 0,
            expires_at_us: 0, node: 0, subject: 0, lease_epoch: 0, write: false, occupied: false },
    })
}
pub(crate) fn validate(mailbox: DsmMailbox, local: NodeId, now_us: u64) -> Result<(), ActorError<core::convert::Infallible>> {
    result(unsafe { ghostos_actor_validate_mailbox(mailbox.destination.raw(), mailbox.range.start,
        mailbox.range.length, mailbox.authority.subject.raw(), mailbox.authority.range.start,
        mailbox.authority.range.length, mailbox.authority.write, mailbox.authority.lease_epoch,
        mailbox.authority.expires_at_us, local.raw(), now_us, cfg!(debug_assertions)) })
}
pub(crate) fn decode(words: [u64; 4]) -> Result<(), ActorError<core::convert::Infallible>> {
    result(unsafe { ghostos_actor_decode_words(words[0], words[1], words[2], words[3]) })
}
pub(crate) fn endpoint_matches(actor: ActorId, endpoint: ActorEndpoint, local: NodeId) -> bool {
    let (kind, endpoint) = kind(endpoint);
    unsafe { ghostos_actor_endpoint_matches(actor.node.raw(), kind, endpoint, local.raw()) }
}
pub(crate) fn register<const ACTORS: usize, const NODES: usize>(
    system: &ActorSystem<ACTORS, NODES>, actor: ActorId, endpoint: ActorEndpoint, generation: u32,
) -> Result<usize, ActorError<core::convert::Infallible>> {
    let entries = entries(&system.directory);
    let (kind, endpoint_id) = kind(endpoint);
    let mut index = 0;
    result(unsafe { ghostos_actor_register(entries.as_ptr(), ACTORS, actor.node.raw(), actor.local,
        kind, endpoint_id, generation, system.local.raw(), &mut index) })?;
    Ok(index)
}
pub(crate) fn find<const ACTORS: usize, const NODES: usize>(
    system: &ActorSystem<ACTORS, NODES>, actor: ActorId,
) -> Result<usize, ActorError<core::convert::Infallible>> {
    let entries = entries(&system.directory);
    let mut index = 0;
    result(unsafe { ghostos_actor_find(entries.as_ptr(), ACTORS, actor.node.raw(), actor.local, &mut index) })?;
    Ok(index)
}
pub(crate) fn add_route<const ACTORS: usize, const NODES: usize>(
    system: &ActorSystem<ACTORS, NODES>, route: DsmMailbox, now_us: u64,
) -> Result<usize, ActorError<core::convert::Infallible>> {
    let routes = routes(&system.routes);
    let mut index = 0;
    result(unsafe { ghostos_actor_add_route(routes.as_ptr(), NODES, route.destination.raw(),
        route.range.start, route.range.length, route.authority.subject.raw(),
        route.authority.range.start, route.authority.range.length, route.authority.write,
        route.authority.lease_epoch, route.authority.expires_at_us, system.local.raw(), now_us,
        cfg!(debug_assertions), &mut index) })?;
    Ok(index)
}
pub(crate) struct SpawnPlan { pub directory: usize, pub route: usize, pub local: bool }
pub(crate) fn prepare<const ACTORS: usize, const NODES: usize>(
    system: &ActorSystem<ACTORS, NODES>, actor: ActorId, image: u128, generation: u32,
) -> Result<SpawnPlan, ActorError<core::convert::Infallible>> {
    let entries = entries(&system.directory);
    let routes = routes(&system.routes);
    let mut directory = 0;
    let mut route = 0;
    let mut local = false;
    result(unsafe { ghostos_actor_prepare(entries.as_ptr(), ACTORS, routes.as_ptr(), NODES,
        actor.node.raw(), actor.local, (image >> 64) as u64, image as u64, generation,
        system.local.raw(), &mut directory, &mut route, &mut local) })?;
    Ok(SpawnPlan { directory, route, local })
}
const _: () = {
    assert!(core::mem::size_of::<Entry>() == 24);
    assert!(core::mem::offset_of!(Entry, kind) == 20);
    assert!(core::mem::size_of::<Route>() == 56);
    assert!(core::mem::offset_of!(Route, write) == 52);
};
unsafe extern "C" {
    fn ghostos_actor_validate_mailbox(destination: u32, range_start: u64, range_length: u64,
        subject: u32, auth_start: u64, auth_length: u64, write: bool, lease_epoch: u32,
        expires_at_us: u64, local: u32, now_us: u64, checked: bool) -> i32;
    fn ghostos_actor_decode_words(source_node: u64, source_local: u64,
        destination_node: u64, destination_local: u64) -> i32;
    fn ghostos_actor_endpoint_matches(actor_node: u32, kind: u8, endpoint: u32, local_node: u32) -> bool;
    fn ghostos_actor_register(entries: *const Entry, count: usize, node: u32, local_id: u64,
        kind: u8, endpoint: u32, generation: u32, local_node: u32, index: *mut usize) -> i32;
    fn ghostos_actor_find(entries: *const Entry, count: usize, node: u32, local_id: u64,
        index: *mut usize) -> i32;
    fn ghostos_actor_add_route(routes: *const Route, count: usize, destination: u32,
        range_start: u64, range_length: u64, subject: u32, auth_start: u64, auth_length: u64,
        write: bool, lease_epoch: u32, expires_at_us: u64, local: u32, now_us: u64,
        checked: bool, index: *mut usize) -> i32;
    fn ghostos_actor_prepare(entries: *const Entry, actor_count: usize, routes: *const Route,
        route_count: usize, actor_node: u32, actor_local: u64, image_high: u64, image_low: u64,
        generation: u32, local_node: u32, directory: *mut usize, route: *mut usize,
        local_spawn: *mut bool) -> i32;
}
