use netbuf::Buf;

pub struct Request<'a> {
    buf: &'a mut Buf,
}
