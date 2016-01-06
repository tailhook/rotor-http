use super::Request;

pub trait RequestCreator {
    fn create(&mut self, &mut Request);
}
