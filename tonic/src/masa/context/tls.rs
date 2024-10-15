///
pub mod client {
    use crate::masa::{ClientStubHooks, RequestHandlerHooks};

    ///
    pub fn get_parent_ctx<'a, C: ClientStubHooks, P: RequestHandlerHooks<C>>() -> Option<&'a P> {
        todo!()
    }
}

///
pub mod server {
    use crate::masa::{ClientStubHooks, RequestHandlerHooks};

    ///
    pub fn set_parent_ctx<'a, C: ClientStubHooks, P: RequestHandlerHooks<C>>(parent_ctx: &'a P) {
        todo!()
    }

    ///
    pub fn reset_parent_ctx<'a, C: ClientStubHooks, P: RequestHandlerHooks<C>>() {
        todo!()
    }
}
