use super::{pool, RequestCreator, PrivContext};

pub struct Pool<R: RequestCreator>(pool::Pool<R>);

pub trait Context<R: RequestCreator> {
    fn get_pool<'x>(&'x mut self) -> &'x mut Pool<R>;
}

impl<R: RequestCreator> Pool<R> {
    pub fn new() -> Pool<R> {
        Pool(pool::Pool::new())
    }
}

impl<R: RequestCreator> Pool<R> {
    fn internal(&mut self) -> &mut pool::Pool<R> {
        &mut self.0
    }
}

impl<R:RequestCreator, T:Context<R>> PrivContext<R> for T {
    fn pool(&mut self) -> &mut pool::Pool<R> {
        self.get_pool().internal()
    }
}
