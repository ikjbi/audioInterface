//! Linux backend: the PulseAudio native protocol, spoken directly over its Unix
//! socket using the pure-Rust `pulseaudio` crate (no libpulse needed).
//!
//! This also works on PipeWire systems, which nearly all modern distros use, since
//! `pipewire-pulse` implements the same protocol.

use super::{AudioBackend, clamp_volume};
use crate::bluetooth::{self, Hints};
use crate::model::{Device, Direction};
use anyhow::{Context, Result, anyhow, bail};
use pulseaudio::protocol::{self, Command, CommandReply, Prop};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// One synchronous protocol connection.
struct Conn {
    sock: BufReader<UnixStream>,
    version: u16,
    seq: u32,
}

impl Conn {
    fn connect() -> Result<Self> {
        let path = pulseaudio::socket_path_from_env()
            .ok_or_else(|| anyhow!("PulseAudio/PipeWire socket not found (is a sound server running?)"))?;
        let stream =
            UnixStream::connect(&path).with_context(|| format!("connect to PulseAudio at {}", path.display()))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut sock = BufReader::new(stream);

        let cookie = pulseaudio::cookie_path_from_env().and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
        let auth =
            protocol::AuthParams { version: protocol::MAX_VERSION, supports_shm: false, supports_memfd: false, cookie };
        protocol::write_command_message(sock.get_mut(), 0, &Command::Auth(auth), protocol::MAX_VERSION)?;
        let (_, reply) = protocol::read_reply_message::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION)
            .context("PulseAudio authentication")?;
        let version = protocol::MAX_VERSION.min(reply.version);

        let mut props = protocol::Props::new();
        props.set(Prop::ApplicationName, CString::new("Audio Interface").unwrap());
        props.set(Prop::ApplicationId, CString::new("com.ikjbi.AudioInterface").unwrap());
        protocol::write_command_message(sock.get_mut(), 1, &Command::SetClientName(props), version)?;
        protocol::read_reply_message::<protocol::SetClientNameReply>(&mut sock, version)?;

        Ok(Conn { sock, version, seq: 1 })
    }

    fn next_seq(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn request<R: CommandReply>(&mut self, cmd: Command) -> Result<R> {
        let seq = self.next_seq();
        protocol::write_command_message(self.sock.get_mut(), seq, &cmd, self.version)?;
        let (_, reply) = protocol::read_reply_message::<R>(&mut self.sock, self.version)?;
        Ok(reply)
    }

    fn ack(&mut self, cmd: Command) -> Result<()> {
        let seq = self.next_seq();
        protocol::write_command_message(self.sock.get_mut(), seq, &cmd, self.version)?;
        protocol::read_ack_message(&mut self.sock)?;
        Ok(())
    }
}

fn cstr(s: &CStr) -> String {
    s.to_string_lossy().into_owned()
}

fn prop<'a>(props: &'a protocol::Props, key: &CStr) -> Option<&'a str> {
    let bytes = props.get_bytes(key)?;
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    std::str::from_utf8(bytes).ok()
}

/// PulseAudio volumes are u32 with `NORM` = 100%. Like `pactl` and
/// pavucontrol, we expose `raw / NORM` as the percentage (clamped to 100%).
fn volume_scalar(cv: &protocol::ChannelVolume) -> Option<f32> {
    let ch = cv.channels();
    if ch.is_empty() {
        return None;
    }
    let avg = ch.iter().map(|v| v.as_u32() as f64).sum::<f64>() / ch.len() as f64;
    Some((avg / protocol::Volume::NORM.as_u32() as f64).clamp(0.0, 1.0) as f32)
}

fn channel_volume(channels: usize, scalar: f32) -> protocol::ChannelVolume {
    let raw = (clamp_volume(scalar) as f64 * protocol::Volume::NORM.as_u32() as f64).round() as u32;
    let mut cv = protocol::ChannelVolume::empty();
    for _ in 0..channels.max(1) {
        cv.push(protocol::Volume::from_u32_clamped(raw));
    }
    cv
}

pub struct PulseBackend {
    conn: Option<Conn>,
    /// Sink name -> its monitor source name (used to meter outputs).
    monitors: HashMap<String, String>,
    /// (name, direction) -> channel count, for building volume structs.
    channels: HashMap<(String, Direction), usize>,
}

impl PulseBackend {
    pub fn connect() -> Result<Self> {
        Ok(PulseBackend { conn: Some(Conn::connect()?), monitors: HashMap::new(), channels: HashMap::new() })
    }

    /// Run `f` on the connection, reconnecting once if the server went away
    /// (e.g. `systemctl --user restart pipewire-pulse`).
    fn with_conn<T>(&mut self, mut f: impl FnMut(&mut Conn) -> Result<T>) -> Result<T> {
        if self.conn.is_none() {
            self.conn = Some(Conn::connect()?);
        }
        match f(self.conn.as_mut().unwrap()) {
            Ok(v) => Ok(v),
            Err(e) if is_io_error(&e) => {
                log::info!("PulseAudio connection lost ({e}); reconnecting");
                self.conn = Some(Conn::connect()?);
                f(self.conn.as_mut().unwrap())
            }
            Err(e) => Err(e),
        }
    }

    fn channels_for(&self, id: &str, direction: Direction) -> usize {
        self.channels.get(&(id.to_string(), direction)).copied().unwrap_or(2)
    }
}

fn is_io_error(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>().is_some()
            || matches!(c.downcast_ref::<protocol::ProtocolError>(), Some(protocol::ProtocolError::Io(_)))
    })
}

impl AudioBackend for PulseBackend {
    fn api_name(&self) -> &'static str {
        "PulseAudio"
    }

    fn devices(&mut self) -> Result<Vec<Device>> {
        let (server, sinks, sources, cards) = self.with_conn(|c| {
            let server: protocol::ServerInfo = c.request(Command::GetServerInfo)?;
            let sinks: protocol::SinkInfoList = c.request(Command::GetSinkInfoList)?;
            let sources: protocol::SourceInfoList = c.request(Command::GetSourceInfoList)?;
            // Card info is only used for Bluetooth profile detection. Don't fail
            // the listing if a server doesn't support it.
            let cards: protocol::CardInfoList = c.request(Command::GetCardInfoList).unwrap_or_default();
            Ok((server, sinks, sources, cards))
        })?;

        let card_profile: HashMap<u32, String> =
            cards.iter().filter_map(|c| Some((c.index, cstr(c.active_profile.as_ref()?)))).collect();
        let default_sink = server.default_sink_name.as_deref().map(cstr);
        let default_source = server.default_source_name.as_deref().map(cstr);

        self.monitors.clear();
        self.channels.clear();
        let mut out = Vec::new();

        let push = |out: &mut Vec<Device>,
                    name: String,
                    description: Option<String>,
                    props: &protocol::Props,
                    card: Option<u32>,
                    direction: Direction,
                    is_default: bool,
                    cv: &protocol::ChannelVolume,
                    muted: bool| {
            let display = description.unwrap_or_else(|| name.clone());
            let profile = card
                .and_then(|i| card_profile.get(&i).cloned())
                .or_else(|| prop(props, c"api.bluez5.profile").map(str::to_string))
                .or_else(|| prop(props, c"bluetooth.protocol").map(str::to_string));
            let hints = Hints {
                name: &display,
                bus: prop(props, c"device.bus"),
                profile: profile.as_deref(),
                ..Default::default()
            };
            let transport = bluetooth::guess_transport(&hints);
            let hands_free = bluetooth::is_hands_free(transport, &hints);
            out.push(Device {
                id: name,
                name: display,
                direction,
                transport,
                is_default,
                volume: volume_scalar(cv),
                muted: Some(muted),
                hands_free,
            });
        };

        for s in &sinks {
            let name = cstr(&s.name);
            if let Some(m) = &s.monitor_source_name {
                self.monitors.insert(name.clone(), cstr(m));
            }
            self.channels.insert((name.clone(), Direction::Output), s.cvolume.channels().len());
            let is_default = default_sink.as_deref() == Some(name.as_str());
            push(
                &mut out,
                name,
                s.description.as_deref().map(cstr),
                &s.props,
                s.card_index,
                Direction::Output,
                is_default,
                &s.cvolume,
                s.muted,
            );
        }
        for s in &sources {
            // Monitor sources are loopbacks of outputs, not real inputs.
            if s.monitor_of_sink_index.is_some() {
                continue;
            }
            let name = cstr(&s.name);
            self.channels.insert((name.clone(), Direction::Input), s.cvolume.channels().len());
            let is_default = default_source.as_deref() == Some(name.as_str());
            push(
                &mut out,
                name,
                s.description.as_deref().map(cstr),
                &s.props,
                s.card_index,
                Direction::Input,
                is_default,
                &s.cvolume,
                s.muted,
            );
        }
        Ok(out)
    }

    fn set_default(&mut self, id: &str, direction: Direction) -> Result<()> {
        let name = CString::new(id)?;
        let exists = self.devices()?.iter().any(|d| d.id == id && d.direction == direction);
        if !exists {
            bail!("no {direction} device named {id}");
        }
        self.with_conn(|c| match direction {
            Direction::Output => c.ack(Command::SetDefaultSink(name.clone())),
            Direction::Input => c.ack(Command::SetDefaultSource(name.clone())),
        })
    }

    fn set_volume(&mut self, id: &str, direction: Direction, volume: f32) -> Result<()> {
        if self.channels.is_empty() {
            self.devices()?;
        }
        let params = protocol::SetDeviceVolumeParams {
            device_index: None,
            device_name: Some(CString::new(id)?),
            volume: channel_volume(self.channels_for(id, direction), volume),
        };
        self.with_conn(|c| match direction {
            Direction::Output => c.ack(Command::SetSinkVolume(params.clone())),
            Direction::Input => c.ack(Command::SetSourceVolume(params.clone())),
        })
    }

    fn set_mute(&mut self, id: &str, direction: Direction, muted: bool) -> Result<()> {
        let params =
            protocol::SetDeviceMuteParams { device_index: None, device_name: Some(CString::new(id)?), mute: muted };
        self.with_conn(|c| match direction {
            Direction::Output => c.ack(Command::SetSinkMute(params.clone())),
            Direction::Input => c.ack(Command::SetSourceMute(params.clone())),
        })
    }

    fn capture_id(&self, id: &str, direction: Direction) -> Option<String> {
        match direction {
            Direction::Input => Some(id.to_string()),
            Direction::Output => self.monitors.get(id).cloned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_roundtrip() {
        let cv = channel_volume(2, 0.5);
        assert_eq!(cv.channels().len(), 2);
        let v = volume_scalar(&cv).unwrap();
        assert!((v - 0.5).abs() < 1e-4);
        assert_eq!(volume_scalar(&channel_volume(1, 2.0)).unwrap(), 1.0);
        assert_eq!(volume_scalar(&protocol::ChannelVolume::empty()), None);
    }
}
