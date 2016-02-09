use time::Duration;


pub trait Context {
    fn byte_timeout(&self) -> Duration {
        Duration::seconds(10)
    }
    fn idle_timeout(&self) -> Duration {
        Duration::seconds(120)
    }
}
