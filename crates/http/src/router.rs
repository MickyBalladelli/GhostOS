use crate::{
    DEFAULT_REQUEST_HEADERS, DEFAULT_RESPONSE_HEADERS, Method, Request, Response,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct WebRights(u16);

impl WebRights {
    pub const NONE: Self = Self(0);
    pub const CALL: Self = Self(1 << 0);
    pub const STREAM: Self = Self(1 << 1);
    pub const ADMIN: Self = Self(1 << 2);
    pub const ALL: Self = Self((1 << 3) - 1);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestContext<'a> {
    pub principal: u64,
    pub request: Request<'a, DEFAULT_REQUEST_HEADERS>,
}

pub type Handler<State> =
    for<'request, 'body> fn(
        &mut State,
        RequestContext<'request>,
        &'body mut [u8],
    ) -> Result<Response<'body, DEFAULT_RESPONSE_HEADERS>, RouteError>;

pub struct Route<State> {
    pub method: Method,
    pub path: &'static str,
    pub required_rights: WebRights,
    pub handler: Handler<State>,
}

impl<State> Copy for Route<State> {}

impl<State> Clone for Route<State> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<State> Route<State> {
    pub const fn new(method: Method, path: &'static str, handler: Handler<State>) -> Self {
        Self {
            method,
            path,
            required_rights: WebRights::CALL,
            handler,
        }
    }

    pub const fn requiring(mut self, rights: WebRights) -> Self {
        self.required_rights = rights;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteError {
    AccessDenied,
    Capacity,
    Duplicate,
    Handler,
    InvalidPath,
    MethodNotAllowed,
    NotFound,
}

pub struct Router<State, const ROUTES: usize> {
    state: State,
    routes: [Option<Route<State>>; ROUTES],
    count: usize,
}

impl<State, const ROUTES: usize> Router<State, ROUTES> {
    pub const fn new(state: State) -> Self {
        Self {
            state,
            routes: [None; ROUTES],
            count: 0,
        }
    }

    pub fn route(&mut self, route: Route<State>) -> Result<(), RouteError> {
        if !valid_route_path(route.path) {
            return Err(RouteError::InvalidPath);
        }
        if self.routes[..self.count]
            .iter()
            .flatten()
            .any(|existing| existing.method == route.method && existing.path == route.path)
        {
            return Err(RouteError::Duplicate);
        }
        let slot = self
            .routes
            .get_mut(self.count)
            .ok_or(RouteError::Capacity)?;
        *slot = Some(route);
        self.count += 1;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub const fn state(&self) -> &State {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    pub fn into_state(self) -> State {
        self.state
    }

    pub fn handle<'body>(
        &mut self,
        principal: u64,
        granted_rights: WebRights,
        request: Request<'_, DEFAULT_REQUEST_HEADERS>,
        body: &'body mut [u8],
    ) -> Result<Response<'body, DEFAULT_RESPONSE_HEADERS>, RouteError> {
        let path = request.path_without_query();
        let mut path_matched = false;
        for route in self.routes[..self.count].iter().flatten().copied() {
            if !matches_path(route.path, path) {
                continue;
            }
            path_matched = true;
            if route.method != request.method {
                continue;
            }
            if !granted_rights.contains(route.required_rights) {
                return Err(RouteError::AccessDenied);
            }
            return (route.handler)(&mut self.state, RequestContext { principal, request }, body);
        }
        if path_matched {
            Err(RouteError::MethodNotAllowed)
        } else {
            Err(RouteError::NotFound)
        }
    }

}

impl<State: Default, const ROUTES: usize> Default for Router<State, ROUTES> {
    fn default() -> Self {
        Self::new(State::default())
    }
}

fn valid_route_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
        && (!path.contains('*') || path.ends_with("/*"))
}

fn matches_path(route: &str, request: &str) -> bool {
    if let Some(prefix) = route.strip_suffix('*') {
        request.starts_with(prefix)
    } else {
        route == request
    }
}
