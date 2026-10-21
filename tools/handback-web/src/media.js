// Reassembles decrypted objects into the files of a hand-back link, like `sr decrypt` +
// collect_media: grouped by the session and sequence the *server* recorded (never the
// uploader's claims), names built from digits only.
import muxjs from 'mux.js';

const dec = new TextDecoder();

/** "SRP1" | BE u32 length | JSON header | content. */
export function decodeEnvelope(buf) {
  if (buf.length < 8 || dec.decode(buf.subarray(0, 4)) !== 'SRP1') throw new Error('not a Témoin object');
  const len = new DataView(buf.buffer, buf.byteOffset + 4, 4).getUint32(0);
  if (8 + len > buf.length) throw new Error('truncated Témoin object');
  return { header: JSON.parse(dec.decode(buf.subarray(8, 8 + len))), content: buf.subarray(8 + len) };
}

const safe = (s) => (String(s || '').match(/[A-Za-z0-9]/g) || []).slice(0, 16).join('') || 'file';

function stamp(createdAt) {
  const d = (String(createdAt || '').match(/[0-9]/g) || []).slice(0, 14).join('');
  return d.length === 14 ? `${d.slice(0, 8)}-${d.slice(8)}` : 'media';
}

// ffmpeg's display matrices for the clockwise rotation the phone recorded (TS has none).
const MATRIX = {
  90: [0, 0x10000, 0, -0x10000, 0, 0, 0, 0, 0x40000000],
  180: [-0x10000, 0, 0, 0, -0x10000, 0, 0, 0, 0x40000000],
  270: [0, -0x10000, 0, 0x10000, 0, 0, 0, 0, 0x40000000],
};

function* boxes(b, start, end) {
  const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
  let o = start;
  while (o + 8 <= end) {
    let size = v.getUint32(o);
    const type = String.fromCharCode(b[o + 4], b[o + 5], b[o + 6], b[o + 7]);
    if (size === 0) size = end - o;
    if (size < 8 || o + size > end) return;
    yield { o, size, type };
    o += size;
  }
}

/** Sets the display rotation of the video track (in place). */
export function rotateVideo(mp4, rotation) {
  const m = MATRIX[((rotation % 360) + 360) % 360];
  if (!m) return mp4;
  const v = new DataView(mp4.buffer, mp4.byteOffset, mp4.byteLength);
  for (const moov of boxes(mp4, 0, mp4.length)) {
    if (moov.type !== 'moov') continue;
    for (const trak of boxes(mp4, moov.o + 8, moov.o + moov.size)) {
      if (trak.type !== 'trak') continue;
      let tkhd = null, handler = '';
      for (const b of boxes(mp4, trak.o + 8, trak.o + trak.size)) {
        if (b.type === 'tkhd') tkhd = b;
        if (b.type === 'mdia') {
          for (const h of boxes(mp4, b.o + 8, b.o + b.size)) {
            if (h.type === 'hdlr') handler = String.fromCharCode(...mp4.subarray(h.o + 16, h.o + 20));
          }
        }
      }
      if (tkhd && handler === 'vide') {
        const at = tkhd.o + (mp4[tkhd.o + 8] === 1 ? 60 : 48);
        m.forEach((x, i) => v.setInt32(at + 4 * i, x));
      }
    }
  }
  return mp4;
}

/** MPEG-TS (Android) to MP4, without re-encoding. */
export function tsToMp4(ts) {
  const t = new muxjs.mp4.Transmuxer({ remux: true, keepOriginalTimestamps: false });
  let init = null;
  const parts = [];
  t.on('data', (seg) => {
    if (!init) init = seg.initSegment;
    parts.push(seg.data);
  });
  t.push(ts);
  t.flush();
  if (!init || !parts.length) throw new Error('video could not be converted');
  const out = new Uint8Array(init.length + parts.reduce((n, p) => n + p.length, 0));
  out.set(init);
  let o = init.length;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}

const TYPES = {
  mpegts: ['mp4', 'video/mp4'],
  'fmp4-init': ['mp4', 'video/mp4'],
  'fmp4-media': ['mp4', 'video/mp4'],
  jpeg: ['jpg', 'image/jpeg'],
  heic: ['heic', 'image/heic'],
};

/**
 * parts: [{ session, seq, header, content }] (session/seq from the server's ledger).
 * Returns [{ name, type, kind, created_at, complete, blob }] sorted by date.
 */
export function assemble(parts) {
  const sessions = new Map();
  for (const p of parts) {
    if (!sessions.has(p.session)) sessions.set(p.session, []);
    sessions.get(p.session).push(p);
  }
  const out = [];
  for (const list of sessions.values()) {
    list.sort((a, b) => a.seq - b.seq);
    const first = list[0].header;
    const [ext, type] = TYPES[first.container] || ['bin', 'application/octet-stream'];
    let expected = 0, missing = false;
    for (const p of list) { if (p.seq !== expected) missing = true; expected = p.seq + 1; }
    const complete = !missing && list[0].seq === 0 && !!list[list.length - 1].header.final;
    let data = new Blob(list.map((p) => p.content));
    let note = '';
    if (first.container === 'mpegts') {
      // Android: concatenated TS segments become one MP4; the phone's rotation goes in the file.
      const ts = new Uint8Array(list.reduce((n, p) => n + p.content.length, 0));
      let o = 0;
      for (const p of list) { ts.set(p.content, o); o += p.content.length; }
      try {
        data = new Blob([rotateVideo(tsToMp4(ts), Number(first.video?.rotation || 0))]);
      } catch (_) {
        note = 'ts'; // conversion failed: hand back the raw stream rather than nothing
      }
    }
    out.push({
      name: `${stamp(first.created_at)}.${note === 'ts' ? 'ts' : ext}`,
      type: note === 'ts' ? 'video/mp2t' : type,
      kind: safe(first.kind),
      created_at: String(first.created_at || ''),
      complete,
      blob: data,
    });
  }
  out.sort((a, b) => (a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : a.name < b.name ? -1 : 1));
  const seen = new Map();
  for (const m of out) {
    const n = (seen.get(m.name) || 0) + 1;
    seen.set(m.name, n);
    if (n > 1) m.name = m.name.replace(/(\.[^.]+)$/, `-${n}$1`);
  }
  return out;
}
