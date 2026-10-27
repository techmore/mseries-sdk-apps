# Local feature stage — October 20, 2026

Signal Pantheon 0.5.1 implements direct voice/video calls, group voice/video call
joining, incoming group-ring prompts, participant video/state, camera/microphone controls, camera selection, self-preview and view-once photos
and videos, alongside the prior timed-message and messaging features.

Local checks cover nine app suites, Java backend tests, Rust tunnel tests, native
photo/video viewing, and synthetic audio/video across two real RingRTC peers with
the production media bridges. The real group client authenticates mock SFU
GET/PUT requests and handles rejection without claiming connection. A native
outgoing-track lifetime fix prevents a group-setup crash. Runtime startup uses an
empty temporary profile; visual checks use private displays, buses and keyrings.

Pairing remains deferred. No real Signal messages, receipts or calls were made;
no hardware camera/microphone, running desktop services or system audio defaults
were changed. Successful live SFU group joining and multi-party media are not
verified. Incoming group rings are locally verified with synthetic envelopes and
the real signaling-only child, including expiry, cancellation, replay suppression
and linked-device responses. Media starts only after acceptance. Live incoming
group delivery remains unverified. Camera selection, switching, missing-camera
recovery and preview teardown pass with synthetic sources; physical camera
discovery/capture remain unverified. Screen sharing, call links,
moderation and the other README limits remain unsupported.
The installation is staged only, not installed to the live desktop.

When pairing is requested, qualify incoming/outgoing direct video, live group
joining/roster/media-key updates, incoming group rings and linked-device dismissal,
camera/microphone controls, and view-once open
sync across linked devices in dedicated test conversations. Local passing checks
do not establish live interoperability or official-client parity.
