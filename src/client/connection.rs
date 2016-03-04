use super::Connection;


impl Connection {
    pub fn is_idle(&self) -> bool {
        self.idle
    }
}
