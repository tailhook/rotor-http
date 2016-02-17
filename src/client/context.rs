use std::time::Duration;


pub trait Context {
    fn byte_timeout(&self) -> Duration {
        Duration::new(10, 0)
    }
    fn idle_timeout(&self) -> Duration {
        Duration::new(120, 0)
    }
}
