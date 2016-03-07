use std::marker::PhantomData;
use std::str::from_utf8;
use std::cmp::min;
use std::fmt;

use rotor::{Scope, Time};
use rotor_stream::{Protocol, StreamSocket};
use rotor_stream::{Intent, Expectation as E, Transport};
use rotor_stream::Buf;
use httparse;

use super::{MAX_HEADERS_SIZE, MAX_HEADERS_NUM, MAX_CHUNK_HEAD};
use super::{Client, Requester, Connection, Task};
use super::head::Head;
use super::request::{Request, state};
use super::head::BodyKind;
use message::{MessageState};
use recvmode::RecvMode;
use headers;
use Version;


#[derive(Debug)]
pub enum BodyProgress {
    /// Buffered fixed-size request (bytes left)
    BufferFixed(usize),
    /// Buffered request till end of input (byte limit)
    BufferEOF(usize),
    /// Buffered request with chunked encoding
    /// (limit, bytes buffered, bytes left for current chunk)
    BufferChunked(usize, usize, usize),
    /// Progressive fixed-size request (size hint, bytes left)
    ProgressiveFixed(usize, u64),
    /// Progressive till end of input (size hint)
    ProgressiveEOF(usize),
    /// Progressive with chunked encoding
    /// (hint, offset, bytes left for current chunk)
    ProgressiveChunked(usize, usize, u64),
}

pub struct Parser<M, S>(M, ParserImpl<M::Requester>, PhantomData<*const S>)
    where M: Client, S: StreamSocket;

enum ParserImpl<M: Requester> {
    Connecting(Time),
    Idle(Time),
    ReadHeaders {
        machine: M,
        request: MessageState,
        is_head: Option<bool>,
    },
    Response {
        progress: BodyProgress,
        machine: M,
        deadline: Time,
        request: MessageState,
    },
    // This state is mostly useful to switch between states easier, but
    // in fact if request is not flushed yet when response is fully received
    // this is actually useful thing
    Flushing(Time),
}

impl<M: Requester> fmt::Debug for ParserImpl<M> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use self::ParserImpl::*;
        match *self {
            Connecting(tm) => {
                fmt.debug_tuple("Connecting").field(&tm).finish()
            }
            Flushing(tm) => {
                fmt.debug_tuple("Flushing").field(&tm).finish()
            }
            Idle(tm) => fmt.debug_tuple("Idle").field(&tm).finish(),
            ReadHeaders { ref request, ref is_head, .. } => {
                fmt.debug_struct("ReadHeaders")
                .field("request", request)
                .field("is_head", is_head)
                .finish()
            }
            Response { ref progress, deadline, ref request, .. } => {
                fmt.debug_struct("Response")
                .field("progress", progress)
                .field("deadline", &deadline)
                .field("request", request)
                .finish()
            },
        }
    }
}

fn scan_headers(is_head: bool, code: u16, headers: &[httparse::Header])
    -> Result<(BodyKind, bool), ()>
{
    /// Implements the body length algorithm for requests:
    /// http://httpwg.github.io/specs/rfc7230.html#message.body.length
    ///
    /// Algorithm:
    ///
    /// 1. For HEAD, 1xx, 204, 304 -- no body
    /// 2. If last transfer encoding is chunked -> Chunked
    /// 3. If Content-Length -> Fixed
    /// 4. Else Eof
    use super::head::BodyKind::*;
    let mut has_content_length = false;
    let mut close = false;
    if is_head || (code > 100 && code < 200) || code == 204 || code == 304 {
        for header in headers.iter() {
            // TODO(tailhook) check for transfer encoding and content-length
            if headers::is_connection(header.name) {
                if header.value.split(|&x| x == b',').any(headers::is_close) {
                    close = true;
                }
            }
        }
        return Ok((Fixed(0), close))
    }
    let mut result = BodyKind::Eof;
    for header in headers.iter() {
        if headers::is_transfer_encoding(header.name) {
            if let Some(enc) = header.value.split(|&x| x == b',').last() {
                if headers::is_chunked(enc) {
                    if has_content_length {
                        // override but don't allow keep-alive
                        close = true;
                    }
                    result = Chunked;
                }
            }
        } else if headers::is_content_length(header.name) {
            if has_content_length {
                // duplicate content_length
                return Err(());
            }
            has_content_length = true;
            if result != Chunked {
                let s = try!(from_utf8(header.value).map_err(|_| ()));
                let len = try!(s.parse().map_err(|_| ()));
                result = Fixed(len);
            } else {
                // tralsfer-encoding has preference and don't allow keep-alive
                close = true;
            }
        } else if headers::is_connection(header.name) {
            if header.value.split(|&x| x == b',').any(headers::is_close) {
                close = true;
            }
        }
    }
    Ok((result, close))
}
fn start_body(mode: RecvMode, body: BodyKind) -> BodyProgress {
    use recvmode::RecvMode::*;
    use super::head::BodyKind::*;
    use self::BodyProgress::*;

    match (mode, body) {
        // The size of Fixed(x) is checked in parse_headers
        (Buffered(_), Fixed(y)) => BufferFixed(y as usize),
        (Buffered(x), Chunked) => BufferChunked(x, 0, 0),
        (Buffered(x), Eof) => BufferEOF(x),
        (Progressive(x), Fixed(y)) => ProgressiveFixed(x, y),
        (Progressive(x), Chunked) => ProgressiveChunked(x, 0, 0),
        (Progressive(x), Eof) => ProgressiveEOF(x),
    }
}

fn parse_headers<M>(buffer: &mut Buf, end: usize,
    proto: M, mut req: Request, is_head: bool,
    scope: &mut Scope<M::Context>)
    -> Result<ParserImpl<M>, ()>
    where M: Requester
{
    let resp = {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS_NUM];
        let (ver, code, reason, headers) = {
            let mut raw = httparse::Response::new(&mut headers);
            match raw.parse(&buffer[..end+4]) {
                Ok(httparse::Status::Complete(x)) => {
                    assert!(x == end+4);
                    let ver = raw.version.unwrap();
                    let code = raw.code.unwrap();
                    (ver, code, raw.reason.unwrap(), raw.headers)
                }
                Ok(_) => unreachable!(),
                Err(_) => {
                    // Anything to do with error?
                    // Should more precise errors be here?
                    return Err(());
                }
            }
        };
        let (body, close) = try!(scan_headers(
            is_head, code, &headers));
        let head = Head {
            version: if ver == 1
                { Version::Http11 } else { Version::Http10 },
            code: code,
            reason: reason,
            headers: headers,
            body_kind: body,
            // For HTTP/1.0 we could implement Connection: Keep-Alive
            // but hopefully it's rare enough to ignore nowadays
            close: close || ver == 0,
        };
        let hdr = proto.headers_received(head, &mut req, scope);
        let (mach, mode, dline) = match hdr {
            Some(triple) => triple,
            None => return Err(()),
        };
        let progress = start_body(mode, body);
        ParserImpl::Response {
            machine: mach,
            deadline: dline,
            progress: progress,
            request: state(req),
        }
    };
    buffer.consume(end+4);
    Ok(resp)
}

impl<M: Client, S: StreamSocket> Parser<M, S> {
    fn finish(cli: M, req: Request,
        scope: &mut Scope<<M::Requester as Requester>::Context>)
        -> Intent<Parser<M, S>>
    {
        if req.is_complete() {
            ParserImpl::Flushing(scope.now() + cli.idle_timeout(scope))
                .intent(cli, scope)
        } else {
            // Response is done before request is sent fully, let's close
            // the connectoin
            // TODO(tailhook) should we return an error?
            return Intent::done();
        }
    }
}

impl<M: Requester> ParserImpl<M> {
    fn wrap<S: StreamSocket, T: Client<Requester=M>>(self, cli: T)
        -> Parser<T, S>
    {
        Parser(cli, self, PhantomData)
    }
    fn intent<S, T>(self, cli: T,
        scope: &mut Scope<<T::Requester as Requester>::Context>)
        -> Intent<Parser<T, S>>
        where S: StreamSocket, T: Client<Requester=M>
    {
        use rotor_stream::Expectation::*;
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        let (exp, dline) = match self {
            Connecting(dline) | Flushing(dline) => (E::Flush(0), dline),
            ReadHeaders { ref machine, ..} => (
                        E::Delimiter(0, b"\r\n\r\n", MAX_HEADERS_SIZE),
                        scope.now() + machine.byte_timeout(scope)),
            Response { ref progress, ref deadline, ref machine, .. } => {
                let exp = match *progress {
                    BufferFixed(x) => Bytes(x),
                    BufferEOF(x) => Bytes(x),
                    BufferChunked(_, off, 0)
                    => Delimiter(off, b"\r\n", off+MAX_CHUNK_HEAD),
                    BufferChunked(_, off, y) => Bytes(off + y),
                    ProgressiveFixed(hint, left)
                    => Bytes(min(hint as u64, left) as usize),
                    ProgressiveEOF(hint) => Bytes(hint),
                    ProgressiveChunked(_, off, 0)
                    => Delimiter(off, b"\r\n", off+MAX_CHUNK_HEAD),
                    ProgressiveChunked(hint, off, left)
                    => Bytes(min(hint as u64, off as u64 +left) as usize)
                };
                (exp, min(*deadline, scope.now() + machine.byte_timeout(scope)))
            }
            Idle(x) => (Sleep, x),
        };
        Intent::of(self.wrap(cli)).expect(exp).deadline(dline)
    }
}

fn maybe_new_request<M: Client, S: StreamSocket>(
    transport: &mut Transport<S>, task: Task<M>,
    scope: &mut Scope<<M::Requester as Requester>::Context>)
    -> Intent<Parser<M, S>>
{
    let (cli, m) = match task {
        Task::Close => return Intent::done(),
        Task::Sleep(cli, deadline) => {
            return ParserImpl::Idle(deadline).intent(cli, scope);
        }
        Task::Request(cli, m) => (cli, m)
    };
    let mut req = Request::new(transport.output());
    match m.prepare_request(&mut req) {
        Some(m) => {
            let deadline = scope.now() + m.byte_timeout(scope);
            Intent::of(Parser(cli, ParserImpl::ReadHeaders {
                    machine: m,
                    is_head: req.1,
                    request: state(req),
                }, PhantomData))
            .expect_delimiter(b"\r\n\r\n", MAX_HEADERS_SIZE)
            .deadline(deadline)
        }
        None => unimplemented!(),
    }
}

impl<M, S> Protocol for Parser<M, S>
    where M: Client, S: StreamSocket
{
    type Context = <M::Requester as Requester>::Context;
    type Socket = S;
    type Seed = M::Seed;
    fn create(seed: Self::Seed, _sock: &mut Self::Socket,
        scope: &mut Scope<Self::Context>)
        -> Intent<Self>
    {
        let cli = M::create(seed, scope);
        let deadline = scope.now() + cli.connect_timeout(scope);
        ParserImpl::Connecting(deadline).intent(cli, scope)
    }
    fn bytes_read(self, transport: &mut Transport<Self::Socket>,
        end: usize, scope: &mut Scope<Self::Context>)
        -> Intent<Self>
    {
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        match self.1 {
            ReadHeaders { machine, request, is_head } => {
                let (inb, outb) = transport.buffers();
                let is_head = is_head.unwrap();
                let hdr = parse_headers(inb, end, machine,
                    request.with(outb), is_head, scope);
                match hdr {
                    Ok(me) => me.intent(self.0, scope),
                    Err(()) => Intent::done(), // Close the connection
                }
            }
            Response { progress, machine, deadline, request }  => {
                let (inp, out) = transport.buffers();
                let mut req = request.with(out);
                let (m, progress) = match progress {
                    BufferFixed(x) => {
                        machine.response_received(
                                  &inp[..x], &mut req, scope);
                        inp.consume(x);
                        return Parser::finish(self.0, req, scope);
                    }
                    BufferEOF(_) => unreachable!(),
                    BufferChunked(limit, off, 0) => {
                        let clen_end = inp[off..end].iter()
                            .position(|&x| x == b';')
                            .map_or(end, |x| x + off);
                        let val_opt = from_utf8(&inp[off..clen_end]).ok()
                            .and_then(|x| u64::from_str_radix(x, 16).ok());
                        match val_opt {
                            Some(0) => {
                                inp.remove_range(off..end+2);
                                machine.response_received(
                                    &inp[..off], &mut req, scope);
                                inp.consume(off);
                                return Parser::finish(self.0, req, scope);
                            }
                            Some(chunk_len) => {
                                if off as u64 + chunk_len > limit as u64 {
                                    inp.consume(end+2);
                                    machine.bad_response(scope);
                                    return Intent::done();
                                }
                                inp.remove_range(off..end+2);
                                (Some(machine),
                                 BufferChunked(limit, off, chunk_len as usize))
                            }
                            None => {
                                inp.consume(end+2);
                                machine.bad_response(scope);
                                return Intent::done();
                            }
                        }
                    }
                    BufferChunked(limit, off, bytes) => {
                        debug_assert!(bytes == end);
                        (Some(machine),
                         BufferChunked(limit, off+bytes, 0))
                    }
                    ProgressiveFixed(hint, mut left) => {
                        let real_bytes = min(inp.len() as u64, left) as usize;
                        let m = machine.response_chunk(
                                    &inp[..real_bytes], &mut req, scope);
                        inp.consume(real_bytes);
                        left -= real_bytes as u64;
                        if left == 0 {
                            m.map(|x| x.response_end(&mut req, scope));
                            return Parser::finish(self.0, req, scope);
                        } else {
                            (m, ProgressiveFixed(hint, left))
                        }
                    }
                    ProgressiveEOF(hint) => {
                        let ln = inp.len();
                        let m = machine.response_chunk(
                                    &inp[..ln], &mut req, scope);
                        (m, ProgressiveEOF(hint))
                    }
                    ProgressiveChunked(hint, off, 0) => {
                        let clen_end = inp[off..end].iter()
                            .position(|&x| x == b';')
                            .map_or(end, |x| x + off);
                        let val_opt = from_utf8(&inp[off..clen_end]).ok()
                            .and_then(|x| u64::from_str_radix(x, 16).ok());
                        match val_opt {
                            Some(0) => {
                                inp.remove_range(off..end+2);
                                let m = machine.response_chunk(
                                    &inp[..off], &mut req, scope);
                                m.map(|m| m.response_end(&mut req, scope));
                                inp.consume(off);
                                return Parser::finish(self.0, req, scope);
                            }
                            Some(chunk_len) => {
                                inp.remove_range(off..end+2);
                                (Some(machine),
                                 ProgressiveChunked(hint, off, chunk_len))
                            }
                            None => {
                                inp.consume(end+2);
                                machine.bad_response(scope);
                                return Intent::done();
                            }
                        }
                    }
                    ProgressiveChunked(hint, off, mut left) => {
                        let ln = min(off as u64 + left, inp.len() as u64) as usize;
                        left -= (ln - off) as u64;
                        if ln < hint {
                            (Some(machine),
                             ProgressiveChunked(hint, ln, left))
                        } else {
                            let m = machine.response_chunk(&inp[..ln],
                                                &mut req, scope);
                            inp.consume(ln);
                            (m, ProgressiveChunked(hint, 0, left))
                        }
                    }
                };
                match m {
                    None => {
                        unimplemented!();
                    }
                    Some(m) => {
                        Response {
                            machine: m,
                            deadline: deadline,
                            progress: progress,
                            request: state(req),
                        }.intent(self.0, scope)
                    }
                }
            }
            // TODO(tailhook) turn this into some error, or log it?
            Idle(..) => Intent::done(),
            Connecting(..) => unreachable!(),
            Flushing(..) => unreachable!(),
        }
    }
    fn bytes_flushed(self, transport: &mut Transport<Self::Socket>,
        scope: &mut Scope<Self::Context>)
        -> Intent<Self>
    {
        use self::ParserImpl::*;
        match self.1 {
            Connecting(..) | Flushing(..) => {
                maybe_new_request(transport,
                    self.0.connection_idle(&Connection {
                        idle: true,
                    }, scope), scope)
            }
            Idle(..) => unreachable!(),
            ReadHeaders {..} => unreachable!(),
            Response { .. }  => {
                unimplemented!();
            }
        }
    }
    fn timeout(self, transport: &mut Transport<Self::Socket>,
        scope: &mut Scope<Self::Context>)
        -> Intent<Self>
    {
        use self::ParserImpl::*;
        match self.1 {
            Idle(..) => {
                // TODO(tailhook) propagate same idle deadline
                maybe_new_request(transport,
                    self.0.timeout(&Connection {
                        idle: true,
                    }, scope), scope)
            }
            s => {
                unimplemented!();
            }
        }
    }
    fn wakeup(self, transport: &mut Transport<Self::Socket>,
        scope: &mut Scope<Self::Context>)
        -> Intent<Self>
    {
        use self::ParserImpl::*;
        match self.1 {
            // skip the event, will child state machine when connected
            me@Connecting(..) => me.intent(self.0, scope),
            // skip the event, will child state machine when connected
            me@Flushing(..) => me.intent(self.0, scope),
            Idle(..) => {
                // TODO(tailhook) propagate same idle deadline
                maybe_new_request(transport,
                    self.0.wakeup(&Connection {
                        idle: true,
                    }, scope), scope)
            }
            _ => {
                unimplemented!();
            }
        }
    }
}
