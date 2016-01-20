use super::Request;

pub trait RequestCreator: Sized {
    fn create(&mut self, &mut Request);
}
