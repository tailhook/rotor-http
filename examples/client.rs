extern crate rotor;
extern crate rotor_http;

use rotor_http::client::{create_pool, RequestCreator, Request};
use rotor_http::method::Method::Get;
use rotor_http::version::HttpVersion::Http11;

struct Context;

struct Req(String);

impl RequestCreator for Req {
    fn create(&mut self, request: &mut Request) {
        request.start(Get, &self.0, Http11);
        request.done_headers().unwrap();
        request.done();
    }
}


fn main() {
    let mut creator = rotor::Loop::new(&rotor::Config::new()).unwrap();
    let mut new_pool = None;
    creator.add_machine_with(|scope| {
        let (fsm, pool) = create_pool::<Req, Context>(scope);
        new_pool = Some(pool);
        Ok(fsm)
    }).unwrap();
    let pool = new_pool.unwrap();
    let loop_inst = creator.instantiate(Context);
    //let lst = TcpListener::bind(&"127.0.0.1:3000".parse().unwrap()).unwrap();
    //loop_inst.add_machine_with(|scope| {
    //    Accept::<Stream<Parser<HelloWorld, _>>, _>::new(lst, scope)
    //}).unwrap();
    loop_inst.run().unwrap();
}
