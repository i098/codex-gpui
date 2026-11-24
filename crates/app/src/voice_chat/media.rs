mod device;
#[cfg(test)]
mod tests;

use anyhow::{Context as _, Result, bail, ensure};
use async_channel::{Receiver, Sender};
use std::{
    collections::VecDeque,
    future::poll_fn,
    io::ErrorKind,
    net::{IpAddr, SocketAddr, UdpSocket},
    task::Poll,
    time::{Duration, Instant},
};
use str0m::{
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
    change::{SdpAnswer, SdpPendingOffer},
    media::{Direction, MediaKind, MediaTime, Mid},
    net::{Protocol, Receive},
};
use tokio::{io::ReadBuf, net::UdpSocket as AsyncSocket};

const SAMPLE_RATE: u32 = 48_000;
const FRAME_SAMPLES: usize = SAMPLE_RATE as usize / 50;
const START_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) enum Update {
    Offer(String),
    Connected,
    Failed(String),
}

pub(super) struct Session(Sender<String>);

impl Session {
    pub(super) fn start() -> Result<(Self, Receiver<Update>)> {
        let (answer, answers) = async_channel::bounded(1);
        let (updates, events) = async_channel::bounded(2);
        std::thread::Builder::new()
            .name("voice-media".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?
                        .block_on(run(&answers, &updates))
                })();
                if let Err(error) = result {
                    let _ = updates.send_blocking(Update::Failed(format!("{error:#}")));
                }
            })?;
        Ok((Self(answer), events))
    }

    pub(super) fn answer(&self, sdp: &str) {
        if sdp.len() <= 64 * 1024 {
            let _ = self.0.try_send(sdp.to_owned());
        }
    }

    pub(super) fn token(&self) -> Sender<String> {
        self.0.clone()
    }

    pub(super) fn matches(&self, token: &Sender<String>) -> bool {
        self.0.same_channel(token)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.0.close();
    }
}

async fn run(answers: &Receiver<String>, updates: &Sender<Update>) -> Result<()> {
    let mut peer = Peer::bind(
        if_addrs::get_if_addrs()?
            .into_iter()
            .filter(|interface| !interface.is_loopback())
            .map(|interface| interface.ip()),
    )?;
    let (offer, pending, mid) = peer.offer()?;
    updates.send(Update::Offer(offer)).await?;
    let Ok(answer) = tokio::time::timeout(START_TIMEOUT, answers.recv()).await? else {
        return Ok(());
    };
    peer.accept_answer(pending, &answer)?;
    let stopped = async { while answers.recv().await.is_ok() {} };
    tokio::pin!(stopped);
    let connected = async {
        loop {
            match peer.next_event().await? {
                Event::Connected => return Ok(()),
                Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                    bail!("Voice connection closed")
                }
                _ => {}
            }
        }
    };
    tokio::select! {
        result = tokio::time::timeout(START_TIMEOUT, connected) => result??,
        _ = &mut stopped => return Ok(()),
    }
    if answers.is_closed() {
        return Ok(());
    }
    let mut devices = device::open()?;
    updates.send(Update::Connected).await?;
    let mut encoder = VoiceEncoder::new()?;
    let mut decoder = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono)?;
    let mut decoded = [0.; FRAME_SAMPLES * 6];
    loop {
        tokio::select! {
            biased;
            _ = &mut stopped => return Ok(()),
            error = devices.errors.recv() => bail!("{}", error?),
            event = peer.next_event() => match event? {
                Event::MediaData(data) if data.mid == mid => {
                    let count = decoder.decode_float(&data.data, &mut decoded, false)?;
                    devices.speaker.push(&decoded[..count]);
                }
                Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                    bail!("Voice connection closed")
                }
                _ => {}
            },
            frame = devices.microphone.recv() => peer.write(mid, &mut encoder, &frame?)?,
        }
    }
}

struct VoiceEncoder {
    opus: opus::Encoder,
    packet: [u8; 1275],
    timestamp: u64,
}

impl VoiceEncoder {
    fn new() -> Result<Self> {
        Ok(Self {
            opus: opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip)?,
            packet: [0; 1275],
            timestamp: 0,
        })
    }
}

struct Socket {
    address: SocketAddr,
    receive: AsyncSocket,
    send: UdpSocket,
}

struct Peer {
    rtc: Rtc,
    sockets: Vec<Socket>,
    events: VecDeque<Event>,
    timeout: Instant,
}

impl Peer {
    fn bind(addresses: impl IntoIterator<Item = IpAddr>) -> Result<Self> {
        let now = Instant::now();
        let mut peer = Self {
            rtc: Rtc::builder()
                .clear_codecs()
                .enable_opus(true, false)
                .set_reordering_size_audio(0)
                .build(now),
            sockets: Vec::new(),
            events: VecDeque::new(),
            timeout: now,
        };
        for ip in addresses {
            if peer.sockets.iter().any(|socket| socket.address.ip() == ip) {
                continue;
            }
            let Ok(socket) = UdpSocket::bind((ip, 0)) else {
                continue;
            };
            let address = socket.local_addr()?;
            socket.set_nonblocking(true)?;
            let send = socket.try_clone()?;
            peer.sockets.push(Socket {
                address,
                receive: AsyncSocket::from_std(socket)?,
                send,
            });
            peer.rtc
                .add_local_candidate(Candidate::host(address, "udp")?);
            peer.drain()?;
        }
        ensure!(
            !peer.sockets.is_empty(),
            "No network interface for voice chat"
        );
        Ok(peer)
    }

    fn offer(&mut self) -> Result<(String, SdpPendingOffer, Mid)> {
        let mut change = self.rtc.sdp_api();
        let mid = change.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None);
        change.add_channel("oai-events".into());
        let (offer, pending) = change.apply().context("Missing voice session offer")?;
        self.drain()?;
        Ok((offer.to_sdp_string(), pending, mid))
    }

    fn accept_answer(&mut self, pending: SdpPendingOffer, sdp: &str) -> Result<()> {
        self.rtc
            .sdp_api()
            .accept_answer(pending, SdpAnswer::from_sdp_string(sdp)?)?;
        self.drain()
    }

    fn write(
        &mut self,
        mid: Mid,
        encoder: &mut VoiceEncoder,
        frame: &[f32; FRAME_SAMPLES],
    ) -> Result<()> {
        let writer = self
            .rtc
            .writer(mid)
            .context("Voice session has no audio track")?;
        let params = writer
            .payload_params()
            .next()
            .context("Voice session did not negotiate Opus")?;
        let (pt, frequency) = (params.pt(), params.spec().clock_rate);
        let len = encoder.opus.encode_float(frame, &mut encoder.packet)?;
        writer.write(
            pt,
            Instant::now(),
            MediaTime::new(encoder.timestamp, frequency),
            &encoder.packet[..len],
        )?;
        encoder.timestamp += FRAME_SAMPLES as u64;
        self.drain()
    }

    fn drain(&mut self) -> Result<()> {
        loop {
            match self.rtc.poll_output()? {
                Output::Transmit(packet) => {
                    let socket = self
                        .sockets
                        .iter()
                        .find(|socket| socket.address == packet.source)
                        .context("Missing socket for voice packet")?;
                    match socket.send.send_to(&packet.contents, packet.destination) {
                        Err(error)
                            if !matches!(
                                error.kind(),
                                ErrorKind::WouldBlock
                                    | ErrorKind::NetworkUnreachable
                                    | ErrorKind::HostUnreachable
                            ) =>
                        {
                            return Err(error.into());
                        }
                        _ => {}
                    }
                }
                Output::Event(event) => self.events.push_back(event),
                Output::Timeout(timeout) => {
                    self.timeout = timeout;
                    return Ok(());
                }
            }
        }
    }

    async fn next_event(&mut self) -> Result<Event> {
        let mut packet = [0; 2048];
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(event);
            }
            let received = poll_fn(|cx| {
                for socket in &self.sockets {
                    let mut buf = ReadBuf::new(&mut packet);
                    if let Poll::Ready(result) = socket.receive.poll_recv_from(cx, &mut buf) {
                        return Poll::Ready(
                            result.map(|source| (buf.filled().len(), source, socket.address)),
                        );
                    }
                }
                Poll::Pending
            });
            let input = tokio::select! {
                result = received => {
                    let (len, source, destination) = result?;
                    let Ok(data) = Receive::new(Protocol::Udp, source, destination, &packet[..len]) else {
                        continue;
                    };
                    let input = Input::Receive(Instant::now(), data);
                    if !self.rtc.accepts(&input) {
                        continue;
                    }
                    input
                }
                _ = tokio::time::sleep_until(self.timeout.into()) => Input::Timeout(Instant::now()),
            };
            self.rtc.handle_input(input)?;
            self.drain()?;
        }
    }
}
