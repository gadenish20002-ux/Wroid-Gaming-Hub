#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use wroid_core::{Point, Resolution};
    use wroid_runtime::{
        ContactId, TouchEngine, TouchEvent, TouchFrame, TouchInjectionError, TouchInjector,
        TouchPhase,
    };

    fn frame(events: impl IntoIterator<Item = TouchEvent>) -> TouchFrame {
        TouchFrame::new(events)
    }

    fn down(id: u16, x: u32, y: u32) -> TouchEvent {
        TouchEvent::new(ContactId::new(id), TouchPhase::Down, Point { x, y })
    }

    struct FailingInjector {
        fails_remaining: usize,
    }

    impl FailingInjector {
        fn once() -> Self {
            Self { fails_remaining: 1 }
        }
    }

    impl TouchInjector for FailingInjector {
        fn inject(&mut self, _frame: &TouchFrame) -> Result<(), TouchInjectionError> {
            if self.fails_remaining > 0 {
                self.fails_remaining -= 1;
                return Err(TouchInjectionError::new("injected failure"));
            }
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct SharedRecordingInjector(Arc<Mutex<Vec<TouchFrame>>>);

    impl SharedRecordingInjector {
        fn last_frame(&self) -> TouchFrame {
            self.0.lock().unwrap().last().unwrap().clone()
        }
    }

    impl TouchInjector for SharedRecordingInjector {
        fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError> {
            self.0.lock().unwrap().push(frame.clone());
            Ok(())
        }
    }

    fn connected_runtime_pair() -> (RuntimeChannelClient, RuntimeChannelServer) {
        let (client, server) = runtime_socket_pair().unwrap();
        (
            RuntimeChannelClient::from_owned_fd(client).unwrap(),
            RuntimeChannelServer::from_owned_fd(server).unwrap(),
        )
    }

    fn spawn_attachment<I: TouchInjector + Send + 'static>(
        server: RuntimeChannelServer,
        resolution: Resolution,
        injector: I,
    ) -> thread::JoinHandle<io::Result<RuntimeAttachmentReport>> {
        thread::spawn(move || {
            let mut engine = TouchEngine::new(injector);
            serve_runtime_attachment(server, resolution, &mut engine, || Ok(()))
        })
    }

    fn ten_contact_down_frame() -> TouchFrame {
        frame((1..=10).map(|id| down(id, id.into(), 20)))
    }

    #[test]
    fn scales_logical_endpoints_into_canonical_axes() {
        let landscape = Resolution {
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            scale_point(Point { x: 0, y: 0 }, landscape).unwrap(),
            Point { x: 0, y: 0 }
        );
        assert_eq!(
            scale_point(Point { x: 1919, y: 1079 }, landscape).unwrap(),
            Point { x: 65535, y: 65535 }
        );
        assert_eq!(scale_axis(960, 1920).unwrap(), 32785);
    }

    #[test]
    fn codec_round_trips_one_event_packet() {
        let request = RuntimeRequest::Frame(frame([down(7, 100, 200)]));
        let decoded = decode_request(
            &encode_request(4, &request).unwrap(),
            Resolution {
                width: 1920,
                height: 1080,
            },
        )
        .unwrap();
        assert_eq!(decoded.sequence, 4);
        assert_eq!(decoded.request, request);
    }

    #[test]
    fn codec_round_trips_ten_events() {
        let request = RuntimeRequest::Frame(frame((1..=10).map(|id| down(id, id.into(), 20))));
        let decoded = decode_request(
            &encode_request(0, &request).unwrap(),
            Resolution {
                width: 1920,
                height: 1080,
            },
        )
        .unwrap();
        assert_eq!(decoded.request, request);
    }

    #[test]
    fn codec_rejects_empty_and_eleven_event_frames() {
        let empty = RuntimeRequest::Frame(frame([]));
        assert_eq!(
            encode_request(0, &empty).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let mut eleven = header(REQUEST_FRAME, 0, 11);
        for id in 1..=11 {
            eleven.extend_from_slice(&encode_event(down(id, 1, 1)));
        }
        assert_eq!(
            decode_request(
                &eleven,
                Resolution {
                    width: 20,
                    height: 30
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn codec_rejects_unknown_version_opcode_and_phase() {
        let request = RuntimeRequest::Frame(frame([down(1, 10, 20)]));
        let mut bytes = encode_request(0, &request).unwrap();
        bytes[0] = 99;
        assert_eq!(
            decode_request(
                &bytes,
                Resolution {
                    width: 20,
                    height: 30
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );

        let mut bytes = encode_request(0, &request).unwrap();
        bytes[2] = 99;
        assert_eq!(
            decode_request(
                &bytes,
                Resolution {
                    width: 20,
                    height: 30
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );

        let mut bytes = encode_request(0, &request).unwrap();
        bytes[HEADER_BYTES + 2] = 99;
        assert_eq!(
            decode_request(
                &bytes,
                Resolution {
                    width: 20,
                    height: 30
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn codec_rejects_short_packet_duplicate_ids_and_coordinates_outside_resolution() {
        assert_eq!(
            decode_request(
                &[0; HEADER_BYTES - 1],
                Resolution {
                    width: 20,
                    height: 30
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
        let duplicate = RuntimeRequest::Frame(frame([down(1, 10, 20), down(1, 11, 21)]));
        assert_eq!(
            decode_request(
                &encode_request(0, &duplicate).unwrap(),
                Resolution {
                    width: 20,
                    height: 30
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
        let outside = RuntimeRequest::Frame(frame([down(1, 20, 29)]));
        assert_eq!(
            decode_request(
                &encode_request(0, &outside).unwrap(),
                Resolution {
                    width: 20,
                    height: 30
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn seqpacket_receive_rejects_repeated_or_out_of_order_sequence() {
        let (client, server) = runtime_socket_pair().unwrap();
        let request = RuntimeRequest::Frame(frame([down(1, 10, 20)]));
        send_raw_packet(client.as_raw_fd(), &encode_request(0, &request).unwrap()).unwrap();
        send_raw_packet(client.as_raw_fd(), &encode_request(0, &request).unwrap()).unwrap();
        let mut server = RuntimeChannelServer::from_owned_fd(server).unwrap();
        assert!(matches!(
            server
                .receive_request(Resolution {
                    width: 20,
                    height: 30
                })
                .unwrap()
                .request,
            RuntimeRequest::Frame(_)
        ));
        assert_eq!(
            server
                .receive_request(Resolution {
                    width: 20,
                    height: 30
                })
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let (client, server) = runtime_socket_pair().unwrap();
        send_raw_packet(client.as_raw_fd(), &encode_request(0, &request).unwrap()).unwrap();
        send_raw_packet(client.as_raw_fd(), &encode_request(2, &request).unwrap()).unwrap();
        let mut server = RuntimeChannelServer::from_owned_fd(server).unwrap();
        server
            .receive_request(Resolution {
                width: 20,
                height: 30,
            })
            .unwrap();
        assert_eq!(
            server
                .receive_request(Resolution {
                    width: 20,
                    height: 30,
                })
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn seqpacket_receive_reports_eof() {
        let (client, server) = runtime_socket_pair().unwrap();
        drop(client);
        assert_eq!(
            RuntimeChannelServer::from_owned_fd(server)
                .unwrap()
                .receive_request(Resolution {
                    width: 20,
                    height: 30
                })
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn seqpacket_receive_rejects_truncated_datagram() {
        let (client, server) = runtime_socket_pair().unwrap();
        send_raw_packet(client.as_raw_fd(), &[0x5a; MAX_PACKET_BYTES + 1]).unwrap();
        let error = RuntimeChannelServer::from_owned_fd(server)
            .unwrap()
            .receive_request(Resolution {
                width: 1920,
                height: 1080,
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("oversized runtime packet"));
    }

    #[test]
    fn failed_uinput_write_is_not_committed_or_acknowledged_as_success() {
        let (mut client, server) = connected_runtime_pair();
        let join = spawn_attachment(
            server,
            Resolution {
                width: 1920,
                height: 1080,
            },
            FailingInjector::once(),
        );
        client.wait_until_ready().unwrap();
        let error = client.inject(&frame([down(1, 100, 200)])).unwrap_err();
        assert!(error.to_string().contains("injected failure"));
        let outcome = join.join().unwrap().unwrap_err();
        assert!(outcome.to_string().contains("injected failure"));
    }

    #[test]
    fn eof_cancels_ten_contacts_in_one_frame() {
        let recording = SharedRecordingInjector::default();
        let (mut client, server) = connected_runtime_pair();
        let join = spawn_attachment(
            server,
            Resolution {
                width: 1920,
                height: 1080,
            },
            recording.clone(),
        );
        client.wait_until_ready().unwrap();
        client.inject(&ten_contact_down_frame()).unwrap();
        drop(client);
        join.join().unwrap().unwrap();
        assert_eq!(recording.last_frame().events().len(), 10);
        assert!(recording
            .last_frame()
            .events()
            .iter()
            .all(|event| event.phase == TouchPhase::Cancel));
    }
}
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use wroid_core::{Point, Resolution};
use wroid_runtime::{
    ContactId, TouchEngine, TouchEvent, TouchFrame, TouchInjectionError, TouchInjector, TouchPhase,
};

pub const RUNTIME_PROTOCOL_VERSION: u16 = 1;
pub const RUNTIME_WORKER_PROTOCOL_GENERATION: u32 = 2;
pub const RUNTIME_WORKER_FD: RawFd = 198;
const MAX_FRAME_EVENTS: usize = 10;
const HEADER_BYTES: usize = 20;
const EVENT_BYTES: usize = 12;
const MAX_PACKET_BYTES: usize = HEADER_BYTES + MAX_FRAME_EVENTS * EVENT_BYTES;
const STARTUP_RESPONSE_TIMEOUT: Duration = Duration::from_secs(300);
const FRAME_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
const SERVER_IDLE_POLL: Duration = Duration::from_millis(250);

const REQUEST_FRAME: u16 = 1;
const REQUEST_FINISH: u16 = 2;
const RESPONSE_READY: u16 = 0x8001;
const RESPONSE_ACK: u16 = 0x8002;
const RESPONSE_ERROR: u16 = 0x8003;

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeRequest {
    Frame(TouchFrame),
    Finish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodedRequest {
    sequence: u64,
    request: RuntimeRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeResponse {
    Ready,
    Ack { sequence: u64 },
    Error { sequence: u64, detail: String },
}

pub fn runtime_socket_pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    // SAFETY: fds points to two writable descriptor slots and the constants are valid on Linux.
    if unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: socketpair succeeded and returned two distinct owned descriptors.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

pub struct RuntimeChannelClient {
    socket: OwnedFd,
    next_sequence: u64,
    ready: bool,
}

impl RuntimeChannelClient {
    pub fn from_owned_fd(fd: OwnedFd) -> io::Result<Self> {
        validate_owned_socket(&fd)?;
        Ok(Self {
            socket: fd,
            next_sequence: 0,
            ready: false,
        })
    }

    pub fn from_owned_fd_for_peer(
        fd: OwnedFd,
        expected_pid: libc::pid_t,
        expected_uid: u32,
    ) -> io::Result<Self> {
        validate_owned_socket(&fd)?;
        let raw_fd = fd.as_raw_fd();
        let mut credentials = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: credentials and length are valid output buffers for SO_PEERCRED.
        if unsafe {
            libc::getsockopt(
                raw_fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                std::ptr::addr_of_mut!(credentials).cast(),
                &mut length,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if length as usize != std::mem::size_of::<libc::ucred>()
            || credentials.pid != expected_pid
            || credentials.uid != expected_uid
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "inherited runtime socket peer is not the expected Wroid daemon",
            ));
        }
        Ok(Self {
            socket: fd,
            next_sequence: 0,
            ready: false,
        })
    }

    pub fn wait_until_ready(&mut self) -> io::Result<()> {
        if self.ready {
            return Ok(());
        }
        match receive_response(self.socket.as_raw_fd(), STARTUP_RESPONSE_TIMEOUT)? {
            RuntimeResponse::Ready => {
                self.ready = true;
                Ok(())
            }
            RuntimeResponse::Error { detail, .. } => Err(io::Error::other(detail)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime channel did not send ready",
            )),
        }
    }

    pub fn finish(&mut self) -> io::Result<()> {
        self.exchange(RuntimeRequest::Finish)
    }

    fn exchange(&mut self, request: RuntimeRequest) -> io::Result<()> {
        if !self.ready {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "runtime channel is not ready",
            ));
        }
        let sequence = self.next_sequence;
        let packet = encode_request(sequence, &request)?;
        send_packet(self.socket.as_raw_fd(), &packet, FRAME_RESPONSE_TIMEOUT)?;
        match receive_response(self.socket.as_raw_fd(), FRAME_RESPONSE_TIMEOUT)? {
            RuntimeResponse::Ack {
                sequence: response_sequence,
            } if response_sequence == sequence => {
                self.next_sequence = self.next_sequence.checked_add(1).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "runtime channel sequence exhausted",
                    )
                })?;
                Ok(())
            }
            RuntimeResponse::Ack { .. } => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime channel response sequence mismatch",
            )),
            RuntimeResponse::Error {
                sequence: response_sequence,
                detail,
            } if response_sequence == sequence => Err(io::Error::other(detail)),
            RuntimeResponse::Error { .. } => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime channel error sequence mismatch",
            )),
            RuntimeResponse::Ready => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime channel sent unexpected ready",
            )),
        }
    }
}

impl TouchInjector for RuntimeChannelClient {
    fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError> {
        self.exchange(RuntimeRequest::Frame(frame.clone()))
            .map_err(|error| TouchInjectionError::new(error.to_string()))
    }
}

pub struct RuntimeChannelServer {
    socket: OwnedFd,
    expected_sequence: u64,
}

impl RuntimeChannelServer {
    pub fn from_owned_fd(fd: OwnedFd) -> io::Result<Self> {
        validate_owned_socket(&fd)?;
        Ok(Self {
            socket: fd,
            expected_sequence: 0,
        })
    }

    pub fn send_startup_error(&mut self, detail: &str) -> io::Result<()> {
        self.send_response(RuntimeResponse::Error {
            sequence: 0,
            detail: detail.to_owned(),
        })
    }

    fn send_ready(&mut self) -> io::Result<()> {
        self.send_response(RuntimeResponse::Ready)
    }

    fn send_response(&mut self, response: RuntimeResponse) -> io::Result<()> {
        let packet = encode_response(&response)?;
        send_packet(self.socket.as_raw_fd(), &packet, FRAME_RESPONSE_TIMEOUT)
    }

    #[cfg(test)]
    fn receive_request(&mut self, resolution: Resolution) -> io::Result<DecodedRequest> {
        self.receive_request_timeout(resolution, None)
    }

    fn receive_request_timeout(
        &mut self,
        resolution: Resolution,
        timeout: Option<Duration>,
    ) -> io::Result<DecodedRequest> {
        let packet = receive_packet(self.socket.as_raw_fd(), timeout)?;
        let request = decode_request(&packet, resolution)?;
        if request.sequence != self.expected_sequence {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime request sequence is repeated or out of order",
            ));
        }
        self.expected_sequence = self.expected_sequence.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime request sequence exhausted",
            )
        })?;
        Ok(request)
    }
}

#[derive(Debug)]
pub struct RuntimeAttachmentReport {
    pub frames_submitted: u64,
    pub peak_contacts: usize,
    pub contacts_cancelled: usize,
}

pub fn serve_runtime_attachment<I: TouchInjector>(
    mut server: RuntimeChannelServer,
    resolution: Resolution,
    engine: &mut TouchEngine<I>,
    mut health_check: impl FnMut() -> io::Result<()>,
) -> io::Result<RuntimeAttachmentReport> {
    if resolution.width == 0 || resolution.height == 0 {
        let error = io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime logical resolution must be non-zero",
        );
        let _ = server.send_startup_error(&error.to_string());
        return Err(error);
    }
    if let Err(error) = server.send_ready() {
        let _ = engine.cancel_all();
        return Err(to_io_error(error));
    }
    let mut report = RuntimeAttachmentReport {
        frames_submitted: 0,
        peak_contacts: 0,
        contacts_cancelled: 0,
    };
    loop {
        let decoded = match server.receive_request_timeout(resolution, Some(SERVER_IDLE_POLL)) {
            Ok(decoded) => decoded,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                if let Err(error) = health_check() {
                    let _ = server.send_response(RuntimeResponse::Error {
                        sequence: server.expected_sequence,
                        detail: error.to_string(),
                    });
                    return cancel_and_return(engine, &mut report, Err(error));
                }
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                return cancel_and_return(engine, &mut report, Ok(()));
            }
            Err(error) => {
                let _ = server.send_response(RuntimeResponse::Error {
                    sequence: server.expected_sequence,
                    detail: error.to_string(),
                });
                return cancel_and_return(engine, &mut report, Err(error));
            }
        };
        match decoded.request {
            RuntimeRequest::Frame(frame) => {
                let frame = match scale_frame(frame, resolution) {
                    Ok(frame) => frame,
                    Err(error) => {
                        let _ = server.send_response(RuntimeResponse::Error {
                            sequence: decoded.sequence,
                            detail: error.to_string(),
                        });
                        return cancel_and_return(engine, &mut report, Err(error));
                    }
                };
                if let Err(error) = engine.submit(frame) {
                    let error = to_io_error(error);
                    let _ = server.send_response(RuntimeResponse::Error {
                        sequence: decoded.sequence,
                        detail: error.to_string(),
                    });
                    return cancel_and_return(engine, &mut report, Err(error));
                }
                report.frames_submitted += 1;
                report.peak_contacts = report
                    .peak_contacts
                    .max(engine.state().active_contact_count());
                if let Err(error) = server.send_response(RuntimeResponse::Ack {
                    sequence: decoded.sequence,
                }) {
                    return cancel_and_return(engine, &mut report, Err(error));
                }
            }
            RuntimeRequest::Finish => {
                report.contacts_cancelled = engine.state().active_contact_count();
                engine.cancel_all().map_err(to_io_error)?;
                server.send_response(RuntimeResponse::Ack {
                    sequence: decoded.sequence,
                })?;
                return Ok(report);
            }
        }
    }
}

fn cancel_and_return<I: TouchInjector>(
    engine: &mut TouchEngine<I>,
    report: &mut RuntimeAttachmentReport,
    outcome: Result<(), io::Error>,
) -> io::Result<RuntimeAttachmentReport> {
    report.contacts_cancelled = engine.state().active_contact_count();
    if let Err(error) = engine.cancel_all() {
        return Err(to_io_error(error));
    }
    outcome.map(|()| RuntimeAttachmentReport {
        frames_submitted: report.frames_submitted,
        peak_contacts: report.peak_contacts,
        contacts_cancelled: report.contacts_cancelled,
    })
}

fn to_io_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn scale_frame(frame: TouchFrame, resolution: Resolution) -> io::Result<TouchFrame> {
    frame
        .events()
        .iter()
        .map(|event| {
            Ok(TouchEvent::new(
                event.contact_id,
                event.phase,
                scale_point(event.position, resolution)?,
            ))
        })
        .collect::<io::Result<Vec<_>>>()
        .map(TouchFrame::new)
}

fn scale_point(point: Point, resolution: Resolution) -> io::Result<Point> {
    Ok(Point {
        x: scale_axis(point.x, resolution.width)?,
        y: scale_axis(point.y, resolution.height)?,
    })
}

fn scale_axis(value: u32, logical_extent: u32) -> io::Result<u32> {
    if logical_extent == 0 || value >= logical_extent {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime touch coordinate is outside the logical resolution",
        ));
    }
    if logical_extent == 1 {
        return Ok(0);
    }
    let denominator = u64::from(logical_extent - 1);
    Ok(((u64::from(value) * 65535 + denominator / 2) / denominator) as u32)
}

fn encode_request(sequence: u64, request: &RuntimeRequest) -> io::Result<Vec<u8>> {
    let (opcode, events) = match request {
        RuntimeRequest::Frame(frame) => (REQUEST_FRAME, frame.events()),
        RuntimeRequest::Finish => (REQUEST_FINISH, &[][..]),
    };
    if opcode == REQUEST_FRAME && (events.is_empty() || events.len() > MAX_FRAME_EVENTS) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime touch frame must contain one through ten events",
        ));
    }
    let mut bytes = header(opcode, sequence, events.len() as u16);
    for event in events {
        bytes.extend_from_slice(&encode_event(*event));
    }
    Ok(bytes)
}

fn encode_event(event: TouchEvent) -> [u8; EVENT_BYTES] {
    let mut bytes = Vec::with_capacity(EVENT_BYTES);
    put_u16(&mut bytes, event.contact_id.get());
    bytes.push(phase_to_byte(event.phase));
    bytes.push(0);
    put_u32(&mut bytes, event.position.x);
    put_u32(&mut bytes, event.position.y);
    bytes
        .try_into()
        .expect("runtime event encoding has fixed length")
}

fn decode_request(bytes: &[u8], resolution: Resolution) -> io::Result<DecodedRequest> {
    if bytes.len() < HEADER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime packet is shorter than its header",
        ));
    }
    if read_u16(bytes, 0) != RUNTIME_PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime protocol version mismatch",
        ));
    }
    if read_u32(bytes, 4) != RUNTIME_WORKER_PROTOCOL_GENERATION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime worker protocol generation mismatch",
        ));
    }
    let opcode = read_u16(bytes, 2);
    let sequence = read_u64(bytes, 8);
    let count = usize::from(read_u16(bytes, 16));
    match opcode {
        REQUEST_FRAME => {
            if count == 0
                || count > MAX_FRAME_EVENTS
                || bytes.len() != HEADER_BYTES + count * EVENT_BYTES
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runtime touch frame has an invalid event count or length",
                ));
            }
            let mut events = Vec::with_capacity(count);
            for index in 0..count {
                let offset = HEADER_BYTES + index * EVENT_BYTES;
                let event = TouchEvent::new(
                    ContactId::new(read_u16(bytes, offset)),
                    byte_to_phase(bytes[offset + 2])?,
                    Point {
                        x: read_u32(bytes, offset + 4),
                        y: read_u32(bytes, offset + 8),
                    },
                );
                scale_point(event.position, resolution)?;
                if events
                    .iter()
                    .any(|previous: &TouchEvent| previous.contact_id == event.contact_id)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "runtime touch frame contains duplicate contact ids",
                    ));
                }
                events.push(event);
            }
            Ok(DecodedRequest {
                sequence,
                request: RuntimeRequest::Frame(TouchFrame::new(events)),
            })
        }
        REQUEST_FINISH if count == 0 && bytes.len() == HEADER_BYTES => Ok(DecodedRequest {
            sequence,
            request: RuntimeRequest::Finish,
        }),
        REQUEST_FINISH => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime finish request has a payload",
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime packet has an unknown opcode",
        )),
    }
}

fn encode_response(response: &RuntimeResponse) -> io::Result<Vec<u8>> {
    match response {
        RuntimeResponse::Ready => Ok(header(RESPONSE_READY, 0, 0)),
        RuntimeResponse::Ack { sequence } => Ok(header(RESPONSE_ACK, *sequence, 0)),
        RuntimeResponse::Error { sequence, detail } => {
            let detail = detail.as_bytes();
            let count = detail.len().min(MAX_PACKET_BYTES - HEADER_BYTES);
            let mut bytes = header(RESPONSE_ERROR, *sequence, count as u16);
            bytes.extend_from_slice(&detail[..count]);
            Ok(bytes)
        }
    }
}

fn receive_response(fd: RawFd, timeout: Duration) -> io::Result<RuntimeResponse> {
    let bytes = receive_packet(fd, Some(timeout))?;
    if bytes.len() < HEADER_BYTES
        || read_u16(&bytes, 0) != RUNTIME_PROTOCOL_VERSION
        || read_u32(&bytes, 4) != RUNTIME_WORKER_PROTOCOL_GENERATION
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid runtime response header",
        ));
    }
    let opcode = read_u16(&bytes, 2);
    let sequence = read_u64(&bytes, 8);
    let count = usize::from(read_u16(&bytes, 16));
    match opcode {
        RESPONSE_READY if count == 0 && bytes.len() == HEADER_BYTES => Ok(RuntimeResponse::Ready),
        RESPONSE_ACK if count == 0 && bytes.len() == HEADER_BYTES => {
            Ok(RuntimeResponse::Ack { sequence })
        }
        RESPONSE_ERROR if bytes.len() == HEADER_BYTES + count => Ok(RuntimeResponse::Error {
            sequence,
            detail: String::from_utf8_lossy(&bytes[HEADER_BYTES..]).into_owned(),
        }),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid runtime response",
        )),
    }
}

fn header(opcode: u16, sequence: u64, count: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_BYTES + usize::from(count) * EVENT_BYTES);
    put_u16(&mut bytes, RUNTIME_PROTOCOL_VERSION);
    put_u16(&mut bytes, opcode);
    put_u32(&mut bytes, RUNTIME_WORKER_PROTOCOL_GENERATION);
    put_u64(&mut bytes, sequence);
    put_u16(&mut bytes, count);
    put_u16(&mut bytes, 0);
    bytes
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn phase_to_byte(phase: TouchPhase) -> u8 {
    match phase {
        TouchPhase::Down => 1,
        TouchPhase::Move => 2,
        TouchPhase::Up => 3,
        TouchPhase::Cancel => 4,
    }
}
fn byte_to_phase(value: u8) -> io::Result<TouchPhase> {
    match value {
        1 => Ok(TouchPhase::Down),
        2 => Ok(TouchPhase::Move),
        3 => Ok(TouchPhase::Up),
        4 => Ok(TouchPhase::Cancel),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime touch event has an unknown phase",
        )),
    }
}

fn send_raw_packet(fd: RawFd, bytes: &[u8]) -> io::Result<()> {
    // SAFETY: bytes is a valid read-only buffer for the duration of send.
    let written = unsafe { libc::send(fd, bytes.as_ptr().cast(), bytes.len(), libc::MSG_NOSIGNAL) };
    if written < 0 {
        return Err(io::Error::last_os_error());
    }
    if written as usize != bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "runtime seqpacket write was partial",
        ));
    }
    Ok(())
}

fn send_packet(fd: RawFd, bytes: &[u8], timeout: Duration) -> io::Result<()> {
    wait_for_fd(fd, libc::POLLOUT, Some(timeout))?;
    send_raw_packet(fd, bytes)
}

fn receive_packet(fd: RawFd, timeout: Option<Duration>) -> io::Result<Vec<u8>> {
    wait_for_fd(fd, libc::POLLIN, timeout)?;
    let mut bytes = [0_u8; MAX_PACKET_BYTES];
    let mut iov = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    // SAFETY: zeroed msghdr is initialized below with a valid iovec.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = std::ptr::addr_of_mut!(iov);
    message.msg_iovlen = 1;
    // SAFETY: message references the writable bytes buffer for this recvmsg call.
    let received = unsafe { libc::recvmsg(fd, &mut message, libc::MSG_CMSG_CLOEXEC) };
    if received < 0 {
        return Err(io::Error::last_os_error());
    }
    if received == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "runtime channel peer closed",
        ));
    }
    if message.msg_flags & libc::MSG_TRUNC != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oversized runtime packet was truncated",
        ));
    }
    Ok(bytes[..received as usize].to_vec())
}

fn wait_for_fd(fd: RawFd, events: i16, timeout: Option<Duration>) -> io::Result<()> {
    let deadline = timeout.map(|duration| Instant::now() + duration);
    loop {
        let timeout_ms = deadline
            .map(|deadline| {
                let remaining =
                    deadline
                        .checked_duration_since(Instant::now())
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::TimedOut,
                                "runtime channel deadline expired",
                            )
                        })?;
                Ok::<_, io::Error>(remaining.as_millis().max(1).min(i32::MAX as u128) as i32)
            })
            .transpose()?;
        let mut pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        // SAFETY: pollfd points to one initialized descriptor entry.
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms.unwrap_or(-1)) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if result == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "runtime channel deadline expired",
            ));
        }
        if pollfd.revents & libc::POLLNVAL != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "runtime channel descriptor is invalid",
            ));
        }
        if pollfd.revents & (events | libc::POLLHUP | libc::POLLERR) != 0 {
            return Ok(());
        }
    }
}

fn validate_owned_socket(fd: &OwnedFd) -> io::Result<()> {
    let raw_fd = fd.as_raw_fd();
    if raw_fd <= libc::STDERR_FILENO {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime channel cannot use a standard I/O descriptor",
        ));
    }
    // SAFETY: stat is writable storage and raw_fd remains owned throughout this function.
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstat(raw_fd, &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if stat.st_mode & libc::S_IFMT != libc::S_IFSOCK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "inherited runtime descriptor is not a Unix socket",
        ));
    }
    let mut socket_type = 0_i32;
    let mut length = std::mem::size_of::<i32>() as libc::socklen_t;
    // SAFETY: socket_type and length are valid output buffers for SO_TYPE.
    if unsafe {
        libc::getsockopt(
            raw_fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            std::ptr::addr_of_mut!(socket_type).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<i32>() || socket_type != libc::SOCK_SEQPACKET {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime channel must be a Unix seqpacket socket",
        ));
    }
    // SAFETY: fcntl reads and updates close-on-exec on this owned descriptor.
    let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(raw_fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
