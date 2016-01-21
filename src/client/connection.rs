use super::Client;


pub struct Connection<C: Client> {
    client: C,
}
