//! The package state after initialisation as bytes (#407, stage 2 step 2): the data of every
//! package, the shared variables area and the shared heap, so that a later run starts from them
//! instead of running the initialisers. Handles inside are plain `u64`s that name heap slots, so
//! the heap is written exactly as it is (slots, generations, free list); nothing is relocated.
//! Valid only for the program the state was made with (type ids in the heap objects).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::heap::{self, Heap};
use crate::{InstData, SharedState};

/// A byte block in a serialised state, packed: the library tables of a verification testbench are
/// megabytes of zeros and of one repeated record, which a restore must not copy.
#[derive(Serialize, Deserialize)]
pub(crate) struct Block(#[serde(with = "packed")] pub Vec<u8>);

#[derive(Serialize)]
pub(crate) struct BlockRef<'a>(#[serde(with = "packed")] pub &'a [u8]);

/// Blocks shorter than this are stored as they are.
const PACK_MIN: usize = 64;
/// The longest block a state may hold when read back (a damaged entry cannot ask for more).
const PACK_MAX: usize = 1 << 30;
/// Runs of zeros shorter than this stay inside a literal.
const ZERO_RUN: usize = 16;
/// Longest period of a repeated pattern looked for, and the shortest stretch worth a repeat.
const MAX_PERIOD: usize = 64;
const MIN_REPEAT: usize = 128;

const LITERAL: u8 = 0;
const ZEROS: u8 = 1;
const REPEAT: u8 = 2;

fn put_len(out: &mut Vec<u8>, mut n: usize) {
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn get_len(b: &[u8], at: &mut usize) -> Option<usize> {
    let mut n = 0usize;
    for shift in (0..64).step_by(7) {
        let x = *b.get(*at)?;
        *at += 1;
        n |= usize::from(x & 0x7f).checked_shl(shift)?;
        if x & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}

/// How far `v[i..]` repeats with period `p` (the number of bytes that equal the ones `p` before).
fn repeats(v: &[u8], i: usize, p: usize) -> usize {
    v[i + p..]
        .iter()
        .zip(&v[i..])
        .take_while(|(a, b)| a == b)
        .count()
}

/// `v` as literals, runs of zeros and repeats of a pattern of up to [`MAX_PERIOD`] bytes.
fn pack(v: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let literal = |out: &mut Vec<u8>, from: usize, to: usize| {
        if to > from {
            out.push(LITERAL);
            put_len(out, to - from);
            out.extend_from_slice(&v[from..to]);
        }
    };
    let (mut i, mut lit) = (0, 0);
    while i < v.len() {
        if v[i] == 0 {
            let run = v[i..].iter().take_while(|&&b| b == 0).count();
            if run >= ZERO_RUN {
                literal(&mut out, lit, i);
                out.push(ZEROS);
                put_len(&mut out, run);
                i += run;
                lit = i;
            } else {
                i += run;
            }
            continue;
        }
        if i % 8 == 0 {
            let best = (4..=MAX_PERIOD)
                .step_by(4)
                .filter(|&p| {
                    i + 2 * p <= v.len() && v[i..i + 8.min(p)] == v[i + p..i + p + 8.min(p)]
                })
                .map(|p| (p, p + repeats(v, i, p)))
                .find(|&(_, n)| n >= MIN_REPEAT);
            if let Some((p, n)) = best {
                literal(&mut out, lit, i);
                out.push(REPEAT);
                put_len(&mut out, p);
                put_len(&mut out, n);
                out.extend_from_slice(&v[i..i + p]);
                i += n;
                lit = i;
                continue;
            }
        }
        i += 1;
    }
    literal(&mut out, lit, v.len());
    out
}

/// [`pack`]'s output back, `n` bytes; `None` when it does not describe exactly `n` bytes.
fn unpack(b: &[u8], n: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; n];
    let (mut at, mut pos) = (0usize, 0usize);
    while at < b.len() {
        let tag = b[at];
        at += 1;
        match tag {
            LITERAL => {
                let len = get_len(b, &mut at)?;
                out.get_mut(pos..pos.checked_add(len)?)?
                    .copy_from_slice(b.get(at..at.checked_add(len)?)?);
                at += len;
                pos += len;
            }
            ZEROS => pos = pos.checked_add(get_len(b, &mut at)?).filter(|&p| p <= n)?,
            REPEAT => {
                let p = get_len(b, &mut at)?;
                let len = get_len(b, &mut at)?;
                if p == 0 || len < p || pos.checked_add(len)? > n {
                    return None;
                }
                out[pos..pos + p].copy_from_slice(b.get(at..at.checked_add(p)?)?);
                at += p;
                // The pattern, doubled until `len` bytes.
                let mut done = p;
                while done < len {
                    let k = done.min(len - done);
                    out.copy_within(pos..pos + k, pos + done);
                    done += k;
                }
                pos += len;
            }
            _ => return None,
        }
    }
    (pos == n).then_some(out)
}

mod packed {
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

    #[derive(Serialize)]
    struct Out<'a>(u64, #[serde(with = "yavs_ir::bytes")] &'a [u8]);

    /// The packed bytes are read in place, not copied out of the entry first.
    #[derive(Deserialize)]
    struct In<'a>(u64, &'a [u8]);

    pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &T, s: S) -> Result<S::Ok, S::Error> {
        let v = v.as_ref();
        if v.len() < super::PACK_MIN {
            return Out(0, v).serialize(s);
        }
        Out(v.len() as u64, &super::pack(v)).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let In(n, bytes) = In::deserialize(d)?;
        if n == 0 {
            return Ok(bytes.to_vec());
        }
        let n = usize::try_from(n)
            .ok()
            .filter(|&n| n <= super::PACK_MAX)
            .ok_or_else(|| de::Error::custom("block too long"))?;
        super::unpack(bytes, n).ok_or_else(|| de::Error::custom("damaged block"))
    }
}

#[derive(Serialize)]
struct Out<'a> {
    data: Vec<BlockRef<'a>>,
    vars: BlockRef<'a>,
    heap: &'a Heap,
    reports: &'a [(u8, String)],
}

#[derive(Deserialize)]
struct In {
    data: Vec<Block>,
    vars: Block,
    heap: Heap,
    reports: Vec<(u8, String)>,
}

impl SharedState {
    /// Print (and count for the exit status) the reports the initialisers made when this state was
    /// made, as they did then; call right after [`restore_state`].
    pub fn replay_init_reports(&self) {
        crate::env::replay_elab(&self.init_reports);
    }

    /// The state after the initialisation of `data`'s packages as bytes for [`restore_state`].
    /// The reason it cannot be reproduced from bytes otherwise: the initialisers did something
    /// besides computing (`tainted`: printed, read the environment or a clock, touched a file,
    /// called foreign code), the simulation has begun, or the heap holds an object that is not
    /// plain data.
    pub fn state_image(&self, data: &[Arc<InstData>]) -> Result<Vec<u8>, String> {
        self.image(data, self.vars.len())
    }

    /// [`SharedState::state_image`] of the first packages only (`data`), written when the
    /// initialisers of the later ones have not run: of the shared variables area, the first
    /// `vars` bytes, which the later packages' objects follow (the lowering allocates them in
    /// package order). [`restore_state_prefix`] reads it.
    pub fn state_image_prefix(
        &self,
        data: &[Arc<InstData>],
        vars: usize,
    ) -> Result<Vec<u8>, String> {
        if vars > self.vars.len() || self.vars[vars..].iter().any(|&b| b != 0) {
            return Err("the later shared variables are not zero".into());
        }
        self.image(data, vars)
    }

    fn image(&self, data: &[Arc<InstData>], vars: usize) -> Result<Vec<u8>, String> {
        if let Some(why) = &self.tainted {
            return Err(why.clone());
        }
        let heap = self.heap.as_deref().filter(|h| h.is_plain(heap::SHARED));
        let plain = !self.started
            && self.procs.is_empty()
            && data.iter().all(|d| d.sigs.is_empty() && d.cov_base == 0);
        let (true, Some(heap)) = (plain, heap) else {
            return Err("the shared state is not plain data".into());
        };
        postcard::to_stdvec(&Out {
            data: data.iter().map(|d| BlockRef(&d.data)).collect(),
            vars: BlockRef(&self.vars[..vars]),
            heap,
            reports: &self.init_reports,
        })
        .map_err(|e| e.to_string())
    }
}

/// [`SharedState::state_image`]'s bytes back: the data of `packages` packages and the shared
/// state, whose variables area has `vars` bytes. `None` when the bytes do not decode or do not
/// fit (another number of packages or area size).
pub fn restore_state(
    bytes: &[u8],
    packages: usize,
    vars: usize,
) -> Option<(Vec<InstData>, SharedState)> {
    restore_state_prefix(bytes, packages, vars, vars)
}

/// [`SharedState::state_image_prefix`]'s bytes back: the data of the first `packages` packages
/// and a shared state whose variables area has `total` bytes, the first `vars` of them as the
/// initialisers left them and the rest zero.
pub fn restore_state_prefix(
    bytes: &[u8],
    packages: usize,
    vars: usize,
    total: usize,
) -> Option<(Vec<InstData>, SharedState)> {
    let i: In = postcard::from_bytes(bytes).ok()?;
    if i.data.len() != packages
        || i.vars.0.len() != vars
        || total < vars
        || !i.heap.is_plain(heap::SHARED)
    {
        return None;
    }
    let data = i
        .data
        .into_iter()
        .map(|b| InstData {
            data: b.0,
            ..Default::default()
        })
        .collect();
    let mut area = i.vars.0;
    area.resize(total, 0);
    let mut st = SharedState::new(area);
    st.heap = Some(Box::new(i.heap));
    st.init_reports = i.reports;
    Some((data, st))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(v: &[u8]) {
        assert_eq!(
            unpack(&pack(v), v.len()).as_deref(),
            Some(v),
            "{} bytes",
            v.len()
        );
    }

    #[test]
    fn state_of_the_first_packages_round_trips() {
        let data: Vec<Arc<InstData>> = (0..2u8)
            .map(|i| {
                Arc::new(InstData {
                    data: vec![i + 1; 100],
                    ..Default::default()
                })
            })
            .collect();
        let mut st = SharedState::new(vec![0; 64]);
        st.vars[..16].copy_from_slice(&[9; 16]);
        let image = st.state_image_prefix(&data, 16).expect("image");
        let (d, back) = restore_state_prefix(&image, 2, 16, 64).expect("restore");
        assert_eq!(d.len(), 2);
        assert_eq!(d[1].data, vec![2u8; 100]);
        assert_eq!(&back.vars[..16], &[9u8; 16]);
        assert_eq!(back.vars.len(), 64);
        assert!(back.vars[16..].iter().all(|&b| b == 0));
        // Another number of packages, or an area that is too small: a miss.
        assert!(restore_state_prefix(&image, 3, 16, 64).is_none());
        assert!(restore_state_prefix(&image, 2, 8, 64).is_none());
        assert!(restore_state_prefix(&image, 2, 16, 8).is_none());
        // Bytes beyond the prefix that are not zero: the later packages have run.
        st.vars[20] = 1;
        assert!(st.state_image_prefix(&data, 16).is_err());
    }

    #[test]
    fn blocks_round_trip() {
        round(&[]);
        round(&[0; 1000]);
        round(&[7; 1000]);
        let mut rec = Vec::new();
        for i in 0..5000u64 {
            rec.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 128, 255, 255, 255, 255, 1, 0, 0, 0]);
            rec.extend_from_slice(&(i % 2).to_le_bytes());
        }
        round(&rec);
        let mut mixed = vec![0u8; 300];
        mixed.extend((0..500).map(|i| (i * 7 % 251) as u8 | 1));
        mixed.extend([0; 40]);
        mixed.extend((0..200).map(|i| (i % 5) as u8 + 1));
        mixed.extend([9; 3]);
        mixed.extend(rec.iter().take(1000));
        mixed.extend([0; 17]);
        round(&mixed);
        // Repeats and zeros take far less than their length.
        assert!(pack(&rec).len() < rec.len() / 50);
        assert!(pack(&[0; 1 << 20]).len() < 16);
    }

    #[test]
    fn damaged_blocks_are_refused() {
        let v: Vec<u8> = [0u8; 100].iter().chain(&[5; 200]).copied().collect();
        let p = pack(&v);
        assert_eq!(unpack(&p, v.len() + 1), None);
        assert_eq!(unpack(&p, v.len() - 1), None);
        assert_eq!(unpack(&p[..p.len() - 1], v.len()), None);
        assert_eq!(unpack(&[9], 5), None);
        assert_eq!(unpack(&[REPEAT, 0, 5], 5), None);
    }
}
