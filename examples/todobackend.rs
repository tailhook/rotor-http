//! A [Todo-Backend](http://todobackend.com/) implementation with rotor-http.
//!
//! Todo-Backed is a showcase to compare different languages and frameworks
//! for building web services. One has to implement a simple todo list
//! that is checked by automatic tests available on the website.
//!
//! NOTE: This example only works with Nightly Rust for now because
//! of serde macros.
//!
//! To run use `cargo run --example todobackend --features nightly`.
//!
//! Now you can edit your own todo list via
//! http://todobackend.com/client/index.html?http://localhost:3000
//! (The web page is just for the frontend, the backend is this Rust
//! program.)

#![cfg_attr(feature="nightly", feature(custom_derive, plugin))]
#![cfg_attr(feature="nightly", plugin(serde_macros))]
#![allow(warnings)]

extern crate rotor;
extern crate rotor_http;
#[cfg(feature="nightly")] extern crate serde;
#[cfg(feature="nightly")] extern crate serde_json;

use std::borrow::Cow;
use std::collections::HashMap;
use std::str::from_utf8;
use std::time::Duration;

use rotor::{Scope, Time};
use rotor::mio::tcp::TcpListener;
use rotor_http::server::{self, Fsm, Head, RecvMode, Response, Server};

/// Represents a single Todo entry.
///
/// The entry is serialized and deserialized by serde without
/// any handwritten code.
#[cfg(feature="nightly")]
#[derive(Serialize, Deserialize, Debug)]
struct Todo {
    url: Option<String>,
    title: String,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    order: u64,
}

#[cfg(feature="nightly")]
impl Todo {
    fn set_url(&mut self, id: u64) {
        self.url = Some(format!("http://localhost:3000/todo/{}", id));
    }

    fn patch(&mut self, patch: TodoPatch) {
        if let Some(title) = patch.title {
            self.title = title;
        }
        if let Some(completed) = patch.completed {
            self.completed = completed;
        }
        if let Some(order) = patch.order {
            self.order = order
        }
    }
}

/// One can patch the title and the status of todos. It is also
/// possible to change the ordering.
#[cfg(feature="nightly")]
#[derive(Deserialize, Debug)]
struct TodoPatch {
    title: Option<String>,
    completed: Option<bool>,
    order: Option<u64>,
}

#[cfg(feature="nightly")]
/// Context is used for global storage.
struct Context {
    /// The last id assigned to a todo entry.
    ///
    /// New ids are incremented by one.
    last_id: u64,
    /// The in memory "database" used for the todo-list.
    database: HashMap<u64, Todo>,
}

#[cfg(feature="nightly")]
impl server::Context for Context {}

#[cfg(feature="nightly")]
trait Database {
    /// Get a unique new id.
    fn id(&mut self) -> u64;
    /// List all existing Todos.
    fn list(&self) -> Vec<&Todo>;
    /// Create a new Todo.
    fn create(&mut self, u64, Todo);
    /// Clear the database.
    fn clear(&mut self);
    /// Get one todo by id.
    fn get(&mut self, u64) -> Option<&Todo>;
    /// Get a mutable reference of a todo.
    ///
    /// This is useful for updating or patching an entry.
    fn get_mut(&mut self, u64) -> Option<&mut Todo>;
    /// Delete an entry.
    fn delete(&mut self, id: u64);
}

#[cfg(feature="nightly")]
impl Database for Context {
    fn id(&mut self) -> u64 {
        self.last_id += 1;
        self.last_id
    }

    fn list(&self) -> Vec<&Todo> {
        self.database.values().collect()
    }

    fn create(&mut self, id: u64, todo: Todo) {
        assert!(self.database.insert(id, todo).is_none());
    }

    fn clear(&mut self) {
        self.database.clear();
    }

    fn get(&mut self, id: u64) -> Option<&Todo> {
        self.database.get(&id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Todo> {
        self.database.get_mut(&id)
    }

    fn delete(&mut self, id: u64) {
        self.database.remove(&id);
    }
}

/// The main server state machine. After parsing the header
/// the type of request determined. Each variant of the
/// enum represents a single action. Some are directly related
/// to todo entries others are for error handling or and CORS.
#[derive(Debug, Clone)]
enum TodoBackend {
    List,
    Create,
    Clear,
    Get(u64),
    Patch(u64),
    Delete(u64),
    Preflight,
    MethodNotAllowed(&'static [u8]),
    NotFound,
}

#[cfg(feature="nightly")]
impl Server for TodoBackend {
    type Context = Context;
    fn headers_received(head: Head, _response: &mut Response,
        scope: &mut Scope<Context>)
        -> Option<(Self, RecvMode, Time)>
    {
        use TodoBackend::*;
        Some((if head.method == "OPTIONS" {
                Preflight
            } else if head.path == "/" {
                match head.method {
                    "GET" => List,
                    "POST" => Create,
                    "DELETE" => Clear,
                    _ => MethodNotAllowed(b"GET, POST, DELETE"),
                }
            } else if head.path.starts_with("/todo/") {
                let id = head.path[6..].parse().unwrap();
                match head.method {
                    "GET" => Get(id),
                    "PATCH" => Patch(id),
                    "DELETE" => Delete(id),
                    _ => MethodNotAllowed(b"GET, PATCH, DELETE"),
                }
            } else {
                NotFound
            }, RecvMode::Buffered(1024), scope.now() + Duration::new(10, 0)))

    }

    fn request_received(self, data: &[u8], response: &mut Response,
        scope: &mut Scope<Context>)
        -> Option<Self>
    {
        use self::TodoBackend::*;
        let text_data = from_utf8(data).unwrap();
        let (status, reason, body) = match self {
            List => (200, "OK", Cow::Owned(serde_json::to_string(&scope.list()).unwrap().into_bytes())),
            Create => {
                let mut todo: Todo = serde_json::from_str(text_data).unwrap();
                let id = scope.id();
                todo.set_url(id);
                let body = serde_json::to_string(&todo).unwrap().into_bytes();
                scope.create(id, todo);
                (201, "Created", Cow::Owned(body))
            }
            Clear => {
                scope.clear();
                (200, "OK", Cow::Borrowed(&b"[]"[..]))
            }
            Get(id) => (200, "OK",
                Cow::Owned(serde_json::to_string(&scope.get(id)).unwrap().into_bytes())),
            Patch(id) => {
                let patch: TodoPatch = serde_json::from_str(text_data).unwrap();
                if let Some(todo) = scope.get_mut(id) {
                    todo.patch(patch);
                    let body = serde_json::to_string(&todo).unwrap().into_bytes();
                    (200, "OK", Cow::Owned(body))
                } else {
                    (404, "Not found", Cow::Borrowed(&b"{}"[..]))
                }
            }
            Delete(id) => {
                scope.delete(id);
                (200, "OK", Cow::Borrowed(&b"{}"[..]))
            }
            Preflight => {
                response.status(200, "OK");
                response.add_length(0).unwrap();
                response.add_header("Access-Control-Allow-Origin", b"*").unwrap();
                response.add_header("Access-Control-Allow-Methods",
                                    b"GET, POST, DELETE, PATCH").unwrap();
                response.add_header("Access-Control-Allow-Headers",
                                    b"Content-Type").unwrap();
                response.add_header("Access-Control-Max-Age", b"60").unwrap();
                response.done_headers().unwrap();
                response.done();
                return None;
            }
            MethodNotAllowed(methods) => {
                let reason = "Method Not Allowed";
                response.status(405, reason);
                response.add_length(reason.len() as u64).unwrap();
                response.add_header("Access-Control-Allow-Origin", b"*").unwrap();
                response.add_header("Content-Type", b"text/plain").unwrap();
                response.add_header("Allow", methods).unwrap();
                response.done_headers().unwrap();
                response.write_body(reason.as_bytes());
                response.done();
                return None;
            }
            NotFound => {
                let reason = "Not Found";
                response.status(404, reason);
                response.add_length(reason.len() as u64).unwrap();
                response.add_header("Access-Control-Allow-Origin", b"*").unwrap();
                response.add_header("Content-Type", b"text/plain").unwrap();
                response.done_headers().unwrap();
                response.write_body(reason.as_bytes());
                response.done();
                return None;
            },
        };
        response.status(status, reason);
        response.add_length(body.len() as u64).unwrap();
        response.add_header("Access-Control-Allow-Origin", b"*").unwrap();
        response.add_header("Content-Type", b"application/json").unwrap();
        response.done_headers().unwrap();
        response.write_body(&body[..]);
        response.done();
        None
    }
    // It is save to leave out `request_chunk` and `request_end` since we
    // only use buffered requests in this example.
    fn request_chunk(self, _chunk: &[u8], _response: &mut Response,
        _scope: &mut Scope<Context>)
        -> Option<Self> { unreachable!(); }
    fn request_end(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<Self> { unreachable!(); }

    fn timeout(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<(Self, Time)>
    {
        unimplemented!();
    }
    fn wakeup(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        unimplemented!();
    }
}

#[cfg(feature="nightly")]
fn main() {
    let event_loop = rotor::Loop::new(&rotor::Config::new()).unwrap();
    let mut loop_inst = event_loop.instantiate(Context {
        last_id: 0,
        database: HashMap::new(),
    });
    let lst = TcpListener::bind(&"127.0.0.1:3000".parse().unwrap()).unwrap();
    loop_inst.add_machine_with(|scope| {
        Fsm::<TodoBackend, _>::new(lst, scope)
    }).unwrap();
    loop_inst.run().unwrap();
}

#[cfg(not(feature="nightly"))]
fn main() { panic!("See NOTE in source. You must use --features nightly.")}
