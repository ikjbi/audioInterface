//! In-app help: one short explanation per feature, opened from the ❓ Help
//! button or the small "?" buttons next to controls.

pub struct Topic {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

pub const TOPICS: &[Topic] = &[
    Topic {
        id: "start",
        title: "Getting started",
        body: "The window lists your inputs (microphones) on the left and outputs (speakers, headphones) on the right. \
The green frame and DEFAULT label mark the device the system uses right now.\n\n\
• Click \"Make default\" to switch the system's default device.\n\
• Drag the slider to change volume / mic gain.\n\
• Right-click a device name for more options.\n\
• The event log at the bottom shows what happened and when.\n\n\
Everything you change here is a real OS setting. It's the same as using Windows Sound settings, macOS Sound \
settings or PulseAudio/PipeWire. Nothing is sent anywhere.",
    },
    Topic {
        id: "volume",
        title: "Volume, mute and balance",
        body: "The slider is the OS volume of the device. For a microphone it is the input gain: how much the OS amplifies \
your voice before apps like Teams receive it.\n\n\
• The 🎤 / 🔊 button mutes and unmutes.\n\
• \"No software volume control\" means the device has no OS volume (common for some USB mics and iPhone \
microphones). Adjust it on the device itself.\n\
• Balance (L–R slider, stereo outputs only) shifts sound left or right. Double-click it to centre. Changing the \
volume keeps the balance.",
    },
    Topic {
        id: "gainlock",
        title: "Gain lock 🔒",
        body: "Click \"Lock\" next to a volume slider to lock it at its current level. If another app (often Teams or Zoom with \
\"automatically adjust microphone sensitivity\") or the OS changes it, the level is put back within half a \
second. A notification tells you it happened.\n\n\
To change a locked level yourself, drag the slider: the lock follows your new value. Click \"Locked\" to \
unlock.\n\nSome devices only accept volume in steps (e.g. 73% becomes 75%). The lock then accepts the nearest \
step instead of fighting the device.",
    },
    Topic {
        id: "preferred",
        title: "Preferred devices and auto-switch",
        body: "Click \"☆ Prefer\" to add a device to your preferred list. The number after ★ is its priority (★1 is the most \
preferred). Reorder the list in Settings.\n\n\
With ⇄ Auto-switch on (top bar), whenever devices connect or disconnect, the most preferred connected \
device becomes the default. Example: prefer your headset first and the laptop speakers second. Turn the \
headset on and it becomes the default; turn it off and the speakers take over.\n\n\
If you pick another device by hand, the app respects that until something is plugged in or out again.",
    },
    Topic {
        id: "meters",
        title: "Level meters",
        body: "The bar shows the signal level in dBFS (0 dB = the loudest possible digital signal).\n\n\
• Green is normal, yellow is loud, red means you're close to distortion.\n\
• The thin line is the peak hold: the loudest moment in the last 1.5 seconds.\n\
• The darker lower band is the RMS (average) level.\n\
• A red block at the right end means clipping: the signal hit 0 dBFS and distorted. Lower the gain.\n\n\
Status dot: ● green = audio arriving, ● yellow = digital silence (only zeros, e.g. a hardware mute switch), \
● red = signal lost (nothing arriving, device stalled or disconnected).\n\n\
The default input is metered automatically, except Bluetooth mics (see \"Bluetooth\"). Use \"▶ Show level\" \
for other devices.",
    },
    Topic {
        id: "stats",
        title: "History, typical level, noise, SNR, LUFS, spectrum",
        body: "Below each metered mic:\n\n\
• History graph: the last 30 seconds (light = peaks, solid = average). If it slowly gets lower while you talk \
normally, your gain or mic is drifting.\n\
• typical: your average speaking level, ignoring pauses. For calls, around -30 to -18 dB is good.\n\
• noise: the background level between words. SNR (signal-to-noise ratio) = typical minus noise. Above 30 \
dB is clean, below 15 dB the other side hears a lot of noise.\n\
• M / S LUFS: loudness as broadcasters measure it (EBU R128). M = the last 0.4 s, S = the last 3 s.\n\
• clips: how many times the signal distorted.\n\
• 📊 opens a spectrum analyzer: how loud each frequency is, from 20 Hz (bass, left) to 20 kHz (treble, right). \
Hum shows up as a spike at 50/60 Hz.",
    },
    Topic {
        id: "bluetooth",
        title: "Bluetooth and hands-free mode",
        body: "Bluetooth headsets have two modes:\n\n\
• High quality (A2DP): great sound, but the microphone is off.\n\
• Hands-free (HFP): the microphone works, but all sound drops to phone-call quality.\n\n\
As soon as any app uses the headset's mic, the headset switches to hands-free mode. That's why music can \
suddenly sound bad during or after a call. The app shows a warning when an output is in hands-free mode, \
and never opens a Bluetooth mic for metering unless you ask (you can allow it in Settings).\n\n\
Tip: use the laptop or a USB mic for calls and keep the headset for listening. Both stay high quality. \
On Linux you can switch the headset's mode in the device menu (right-click → Profile).",
    },
    Topic {
        id: "recording",
        title: "Recording a call and the drop report",
        body: "⏺ Record logs a device once per second: level, OS gain, mute, signal status, Bluetooth mode, which app is \
using it, and every event. Optionally it also records the audio (\"Include audio\" in the Recordings window).\n\n\
When you stop (■ Stop), you get a report:\n\
• your speaking level at the start (baseline);\n\
• any period where it dropped by 6 dB or more for 30 s or longer;\n\
• the reason: if the OS gain was lowered at that time, an app did it (Teams' automatic mic adjustment is \
the usual suspect, so lock the gain). If the gain didn't change, the cause is acoustic or hardware: mic \
position, headset fit, Bluetooth link quality or battery saving.\n\
• gain changes, signal losses, muted periods and hands-free switches, with times.\n\
• when an app (e.g. Teams) starts or stops using the mic. A gain change right when a call starts or \
ends is explained as the app setting or restoring its level, not counted as a problem. (Teams' \
automatic mic adjustment often raises the gain during a call and puts it back afterwards.)\n\n\
Open ⏺ Recordings to see past sessions with a chart: green = your level, blue = OS gain, red areas = \
drops. Hover the chart to read exact values. Files are saved in Documents/Audio Interface Recordings \
(a CSV you can open in Excel, plus a report and the WAV).",
    },
    Topic {
        id: "autorecord",
        title: "Auto-record calls",
        body: "In the ⏺ Recordings window, turn on \"Auto-record calls\". The app watches your microphones, and when another \
app starts using one (Teams joins a call), recording starts automatically. It stops 10 seconds after the \
app lets go of the mic.\n\n\
Enter \"teams\" in \"only for apps containing\" to record only Teams calls. Leave it empty to record any \
app. You don't have to remember to press Record before the problem happens.",
    },
    Topic {
        id: "apps",
        title: "\"In use by\" and the per-app mixer",
        body: "The blue ● label on a device shows which apps are using it right now (e.g. \"● Microsoft Teams\" on your \
mic during a call). On macOS this needs macOS 14 or newer.\n\n\
On Windows and Linux, the \"Apps\" section under a device lists each app playing or recording through \
it, with its own volume slider and mute. Use it to turn down one app without touching the others. \
macOS has no public way to change per-app volume, so it only shows the app names there.",
    },
    Topic {
        id: "profiles",
        title: "Profiles",
        body: "🎚 Profiles saves your current setup (default input and output, volumes, mutes and gain locks) under a name, \
e.g. \"Calls\" and \"Music\". Choose a profile to restore it in one click, from the menu, the tray icon, or \
the command line (audio-interface profile apply calls).\n\n\
If a device from the profile isn't connected, the rest is still applied and you're told what was missing.",
    },
    Topic {
        id: "hotkeys",
        title: "Hotkeys and tray icon",
        body: "Global hotkeys work in any app:\n\
• Ctrl+Alt+M: mute / unmute your microphone\n\
• Ctrl+Alt+O: switch to the next output (preferred ones first)\n\
• Ctrl+Alt+I: switch to the next input\n\
Change them in Settings (on a Mac, Ctrl is ⌃; write CmdOrCtrl for ⌘). On Linux they work in X11 sessions.\n\n\
On Windows and macOS there's also a tray / menu-bar icon: switch devices, mute, apply profiles and start \
or stop recording without opening the window. Closing the window keeps the app running there (change \
this in Settings), and \"Quit\" in the tray menu exits.",
    },
    Topic {
        id: "tools",
        title: "Test tools",
        body: "Right-click a device name:\n\
• Output → 🔊 Test tone: a short beep on the left, the right or both channels, to check which speaker is which.\n\
• Input → 🎙 Mic test: records 5 seconds, then plays them back so you hear exactly what others hear.\n\
• Input → 🎧 Listen to this mic: hear yourself live. Use headphones: with speakers it will howl (feedback).",
    },
    Topic {
        id: "devicemenu",
        title: "Device menu (right-click a name)",
        body: "• ✏ Rename: give a device a short name, e.g. \"Headset (WH-1000XM4 Hands-Free AG Audio)\" → \"Headset mic\".\n\
• 🙈 Hide: remove devices you never use from the list. Restore them in Settings.\n\
• 📞 Use for calls only (Windows): Windows keeps a separate default for communication apps. Use it to send \
Teams/Zoom to your headset while music stays on the speakers. The CALLS label shows which device it is.\n\
• ⏱ Sample rate (macOS): switch the device between e.g. 44.1 and 48 kHz.\n\
• 🎛 Profile (Linux): switch the hardware mode, e.g. Bluetooth high quality ↔ headset.\n\
• 📋 Copy device ID: for scripts and the command line.",
    },
    Topic {
        id: "privacy",
        title: "Privacy and files",
        body: "Everything stays on your computer. The app has no accounts, no tracking and no network use.\n\n\
• Meters need microphone permission. On a Mac: System Settings → Privacy & Security → Microphone. On \
Windows: Settings → Privacy & security → Microphone → \"Let desktop apps access your microphone\".\n\
• Audio is only written to disk when you record with \"Include audio\" on.\n\
• Recordings: Documents/Audio Interface Recordings. Settings: see the path at the bottom of Settings.",
    },
    Topic {
        id: "trouble",
        title: "Troubleshooting",
        body: "• \"Meter unavailable … taking long to open\": the OS is asking for (or blocking) microphone permission. See \
\"Privacy and files\".\n\
• The meter shows ● red / \"signal lost\": the device stopped sending audio. Reconnect it, or check whether a \
Bluetooth headset switched modes.\n\
• \"Digital silence\": the device sends only zeros. Look for a mute switch on the headset or mic.\n\
• Music sounds bad after a call: the Bluetooth headset is still in hands-free mode. Close the app that uses its \
mic, or switch your call mic to another device.\n\
• Report a problem: Settings → Support → \"Copy diagnostics\" and paste it into an issue on GitHub.",
    },
];

pub fn topic(id: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn topics_are_unique_and_filled() {
        let mut ids: Vec<_> = super::TOPICS.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), super::TOPICS.len());
        assert!(super::TOPICS.iter().all(|t| t.body.len() > 80));
        assert!(super::topic("gainlock").is_some());
    }
}
