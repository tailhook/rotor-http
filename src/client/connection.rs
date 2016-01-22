use super::Client;

use rotor::mio::tcp::TcpStream;


pub struct Connection<R: Client> {
    client: R,
    conn: TcpStream,
}

impl<R: Client> Connection<R> {
    pub fn new(cli: R, conn: TcpStream) -> Connection<R> {
        Connection {
            client: cli,
            conn: conn,
        }
    }
}
