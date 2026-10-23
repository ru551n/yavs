//! Differential tests against sim1 goldens (golden/sim1-{2008,2019}.txt,
//! produced by golden/gen.py). Each golden line is a function, its
//! arguments and sim1's output (or ERROR when the case aborted sim1).
//!
//! Known, intended deviations from sim1 are listed in
//! golden/sim1-deviations.txt with the reason; one rule is applied in code:
//! a failed READ leaves the line unchanged in yavs, so for a `g=false`
//! golden the expected rest of line is the whole input.

use std::collections::HashMap;
use yavs_std::env;
use yavs_std::image::*;
use yavs_std::textio::{self, Side};
use yavs_std::types::*;
use yavs_std::value::*;

type Out = Result<Vec<u8>, String>;

fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            match b[i + 1] {
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'x' => {
                    out.push(u8::from_str_radix(&s[i + 2..i + 4], 16).unwrap());
                    i += 4;
                }
                c => panic!("bad escape \\{}", c as char),
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

fn e1() -> EnumType {
    EnumType {
        lits: [
            "Foo",
            "\\Bar\\",
            "'x'",
            "bAz",
            "\\a\\\\b\\",
            "Q",
            "'Q'",
            "\\X\\",
        ]
        .iter()
        .map(|s| match s.as_bytes() {
            [b'\'', c, b'\''] => EnumLit::Char(*c),
            _ => EnumLit::ident(s),
        })
        .collect(),
    }
}

fn dist() -> PhysicalType {
    PhysicalType::new(&[
        ("nm", 1),
        ("um", 1_000),
        ("mm", 1_000_000),
        ("m", 1_000_000_000),
        ("Inch", 25_400_000),
        ("\\Ft\\", 304_800_000),
    ])
}

const DIST_BOUNDS: Bounds = Bounds {
    low: -1_000_000_000,
    high: 1_000_000_000,
};

fn real(s: &str) -> f64 {
    real_value(s.as_bytes(), RealBounds::REAL).unwrap()
}

fn int(s: &str) -> i64 {
    s.parse().unwrap()
}

fn side(s: &str) -> Side {
    if s == "left" { Side::Left } else { Side::Right }
}

fn unit(s: &str) -> i64 {
    TIME_UNITS.iter().find(|u| u.0 == s).unwrap().1
}

fn bits(s: &str) -> Vec<bool> {
    s.bytes().map(|c| c == b'1').collect()
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn e<T>(r: Result<T, impl std::fmt::Display>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

/// `g=... v=... rest=...` of a READ case.
fn read_case(line: &[u8], f: impl FnOnce(&mut &[u8]) -> Out) -> Out {
    let mut l = line;
    Ok(match f(&mut l) {
        Ok(v) => cat(&[b"g=true v=", &v, b" rest=", l]),
        Err(_) => cat(&[b"g=false rest=", l]),
    })
}

fn tm() -> PhysicalType {
    time_type()
}

// Not code evaluation: dispatches a golden case to the yavs-std function under test.
fn eval(rev: Revision, func: &str, a: &[&str]) -> Out {
    let ib = rev.integer_bounds();
    let t = tm();
    let ws = |f: &dyn Fn(&mut Vec<u8>)| -> Out {
        let mut l = Vec::new();
        f(&mut l);
        Ok(l)
    };
    let wr = |f: &dyn Fn(&mut Vec<u8>) -> Result<(), yavs_std::error::ConvError>| -> Out {
        let mut l = Vec::new();
        e(f(&mut l))?;
        Ok(l)
    };
    Ok(match func {
        "int_image" | "int_to_string" => integer_image(int(a[0])),
        "real_image" | "real_to_string" => real_image(real(a[0])),
        "real_digits" => e(real_to_string_digits(real(a[0]), int(a[1]) as usize))?,
        "real_format" => e(real_to_string_format(real(a[0]), &unescape(a[1])))?,
        "time_image" | "time_to_string" => physical_image(int(a[0]), &t),
        "time_unit" => e(physical_to_string_unit(int(a[0]), &t, unit(a[1])))?,
        "time_unit_odd" => {
            let u = e(physical_value(a[1].as_bytes(), &t, Bounds::I64))?;
            e(physical_to_string_unit(int(a[0]), &t, u))?
        }
        "dist_image" => physical_image(int(a[0]), &dist()),
        "e1_image" => enum_image(&e1().lits[int(a[0]) as usize]),
        "e1_to_string" => enum_to_string(&e1().lits[int(a[0]) as usize]),
        "char_image" => character_image(int(a[0]) as u8),
        "char_to_string" => character_to_string(int(a[0]) as u8),
        "bool_image" | "bool_to_string" => boolean_image(a[0] == "true"),
        "bit_image" => enum_image(&bit_type().lits[int(a[0]) as usize]),
        "bit_to_string" => enum_to_string(&bit_type().lits[int(a[0]) as usize]),
        "sev_image" => a[0].as_bytes().to_vec(),
        "bv_to_string" | "bv_to_bstring" => bits_to_string(&bits(a[0])),
        "bv_to_hstring" => bits_to_hstring(&bits(a[0])),
        "bv_to_ostring" => bits_to_ostring(&bits(a[0])),
        "justify" => textio::justify(&unescape(a[0]), side(a[1]), int(a[2]) as usize),
        "int_value" => integer_image(e(integer_value(&unescape(a[0]), ib))?),
        "real_value" => real_image(e(real_value(&unescape(a[0]), RealBounds::REAL))?),
        "time_value" => physical_image(e(physical_value(&unescape(a[0]), &t, Bounds::I64))?, &t),
        "dist_value" => physical_image(
            e(physical_value(&unescape(a[0]), &dist(), DIST_BOUNDS))?,
            &dist(),
        ),
        "e1_value" => enum_image(&e1().lits[e(enum_value(&unescape(a[0]), &e1()))?]),
        "char_value" => character_image(e(character_value(&unescape(a[0])))?),
        "bool_value" => boolean_image(e(boolean_value(&unescape(a[0])))?),
        "bit_value" => enum_image(&bit_type().lits[e(enum_value(&unescape(a[0]), &bit_type()))?]),
        "sev_value" => {
            let ty = severity_level_type();
            enum_image(&ty.lits[e(enum_value(&unescape(a[0]), &ty))?])
        }
        "read_int" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(integer_image(e(textio::read_integer(l, ib))?))
            });
        }
        "read_real" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(real_image(e(textio::read_real(l, RealBounds::REAL))?))
            });
        }
        "read_time" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(physical_image(
                    e(textio::read_physical(l, &t, Bounds::I64))?,
                    &t,
                ))
            });
        }
        "read_bool" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(boolean_image(e(textio::read_boolean(l))?))
            });
        }
        "read_bit" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(if e(textio::read_bit(l))? {
                    b"'1'".to_vec()
                } else {
                    b"'0'".to_vec()
                })
            });
        }
        "read_char" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(character_image(e(textio::read_character(l))?))
            });
        }
        "read_str" => {
            return read_case(&unescape(a[0]), |l| {
                e(textio::read_string(l, int(a[1]) as usize))
            });
        }
        "read_bv" | "bread" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(bits_to_string(&e(textio::read_bit_vector(
                    l,
                    int(a[1]) as usize,
                ))?))
            });
        }
        "hread" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(bits_to_string(&e(textio::hread(l, int(a[1]) as usize))?))
            });
        }
        "oread" => {
            return read_case(&unescape(a[0]), |l| {
                Ok(bits_to_string(&e(textio::oread(l, int(a[1]) as usize))?))
            });
        }
        "sread" => {
            let line = unescape(a[0]);
            let mut l = &line[..];
            let v = textio::sread(&mut l, int(a[1]) as usize);
            cat(&[
                b"n=",
                v.len().to_string().as_bytes(),
                b" v=",
                &v,
                b" rest=",
                l,
            ])
        }
        "read_int_seq" => {
            let line = unescape(a[0]);
            let mut l = &line[..];
            let mut parts = Vec::new();
            for _ in 0..4 {
                parts.push(e(textio::read_integer(&mut l, ib))?.to_string());
            }
            cat(&[parts.join(",").as_bytes(), b" rest=", l])
        }
        "write_int" => {
            ws(&|l| textio::write_integer(l, int(a[0]), side(a[1]), int(a[2]) as usize))?
        }
        "write_real" => wr(&|l| {
            textio::write_real(
                l,
                real(a[0]),
                side(a[1]),
                int(a[2]) as usize,
                int(a[3]) as usize,
            )
        })?,
        "write_real_format" => wr(&|l| textio::write_real_format(l, real(a[0]), &unescape(a[1])))?,
        "write_time" => wr(&|l| {
            textio::write_physical(l, int(a[0]), &t, side(a[1]), int(a[2]) as usize, unit(a[3]))
        })?,
        "write_str" | "swrite" => {
            ws(&|l| textio::write_string(l, &unescape(a[0]), side(a[1]), int(a[2]) as usize))?
        }
        "write_bit" => ws(&|l| textio::write_bit(l, a[0] == "1", side(a[1]), int(a[2]) as usize))?,
        "write_bool" => {
            ws(&|l| textio::write_boolean(l, a[0] == "true", side(a[1]), int(a[2]) as usize))?
        }
        "write_char" => {
            ws(&|l| textio::write_character(l, int(a[0]) as u8, side(a[1]), int(a[2]) as usize))?
        }
        "write_bv" | "bwrite" => {
            ws(&|l| textio::write_bit_vector(l, &bits(a[0]), side(a[1]), int(a[2]) as usize))?
        }
        "hwrite" => ws(&|l| textio::hwrite(l, &bits(a[0]), side(a[1]), int(a[2]) as usize))?,
        "owrite" => ws(&|l| textio::owrite(l, &bits(a[0]), side(a[1]), int(a[2]) as usize))?,
        "write_append" => ws(&|l| {
            textio::write_string(l, b"ab", Side::Right, 0);
            textio::write_integer(l, 12, Side::Right, 4);
            textio::write_bit(l, true, Side::Right, 0);
        })?,
        "readline" => return readline_case(&unescape(a[0])),
        "writeline" => return writeline_case(&a.iter().map(|s| unescape(s)).collect::<Vec<_>>()),
        "resolution_limit" => physical_image(
            env::resolution_limit(yavs_std::arith::TimeResolution::Fs),
            &t,
        ),
        "vhdl_version" => env::vhdl_version(rev).as_bytes().to_vec(),
        // The golden values come from sim1 on Linux; on Windows the separator is `\`.
        "dir_separator" if cfg!(windows) => {
            assert_eq!(env::DIR_SEPARATOR, "\\");
            b"/".to_vec()
        }
        "dir_separator" => env::DIR_SEPARATOR.as_bytes().to_vec(),
        "gmtime_to_string" => e(env::time_record_to_string(
            &e(env::gmtime(real(a[0])))?,
            int(a[1]) as u32,
        ))?,
        "gmtime_fields" => {
            let r = e(env::gmtime(real(a[0])))?;
            format!(
                "{} {} {} {} {} {} {} {} {}",
                r.year,
                r.month,
                r.day,
                r.hour,
                r.minute,
                r.second,
                r.microsecond,
                format!("{:?}", r.weekday).to_lowercase(),
                r.dayofyear
            )
            .into_bytes()
        }
        "epoch_gmtime" => real_image(e(env::epoch(&e(env::gmtime(real(a[0])))?))?),
        "trec_plus" => e(env::time_record_to_string(
            &e(env::add_seconds(&e(env::gmtime(real(a[0])))?, real(a[1])))?,
            6,
        ))?,
        "trec_minus" => e(env::time_record_to_string(
            &e(env::sub_seconds(&e(env::gmtime(real(a[0])))?, real(a[1])))?,
            6,
        ))?,
        "trec_diff" => real_image(e(env::diff_seconds(
            &e(env::gmtime(real(a[0])))?,
            &e(env::gmtime(real(a[1])))?,
        ))?),
        "time_to_seconds" => real_image(env::time_to_seconds(int(a[0]))),
        "seconds_to_time" => physical_image(e(env::seconds_to_time(real(a[0])))?, &t),
        // Tool identity differs from sim1 by design.
        f if f.starts_with("tool_") => return Err("tool".into()),
        f => panic!("no evaluator for golden function {f}"),
    })
}

/// A file path no other case uses: the tests run in parallel in one process.
fn scratch(kind: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{kind}-{}-{n}.txt", std::process::id()))
}

fn readline_case(data: &[u8]) -> Out {
    let p = scratch("rl");
    std::fs::write(&p, data).unwrap();
    let mut f = yavs_std::file::TextFile::open(
        p.to_str().unwrap().as_bytes(),
        yavs_std::file::FileOpenKind::Read,
    )
    .map_err(|s| format!("{s:?}"))?;
    let mut acc = Vec::new();
    while !e(f.endfile())? {
        let l = e(f.readline())?;
        acc.extend(format!("<{}:", l.len()).bytes());
        for c in l {
            if !(32..=126).contains(&c) {
                acc.extend(format!("\\{c}").bytes());
            } else {
                acc.push(c);
            }
        }
        acc.push(b'>');
    }
    Ok(acc)
}

fn writeline_case(lines: &[Vec<u8>]) -> Out {
    use yavs_std::file::{FileOpenKind, TextFile};
    let p = scratch("wl");
    let mut f = TextFile::open(p.to_str().unwrap().as_bytes(), FileOpenKind::Write)
        .map_err(|s| format!("{s:?}"))?;
    let mut empty = true;
    for l in lines {
        let mut l = l.clone();
        e(f.writeline(&mut l))?;
        empty &= l.is_empty();
    }
    e(f.close())?;
    let mut s = format!("null={} bytes=", !empty).into_bytes();
    for b in std::fs::read(&p).unwrap() {
        s.extend(format!("{b} ").bytes());
    }
    Ok(s)
}

/// Deviations: (revision or "*", func, args) -> expected output.
fn deviations() -> HashMap<(String, String, Vec<String>), String> {
    let text = include_str!("../golden/sim1-deviations.txt");
    let mut m = HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        // rev, func, args..., expected, reason
        assert!(f.len() >= 4, "bad deviation line: {line}");
        let n = f.len();
        m.insert(
            (
                f[0].to_string(),
                f[1].to_string(),
                f[2..n - 2].iter().map(|s| s.to_string()).collect(),
            ),
            f[n - 2].to_string(),
        );
    }
    m
}

fn show(o: &[u8]) -> String {
    o.iter().map(|&c| c as char).collect()
}

fn run(rev: Revision, file: &str) -> usize {
    let data = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("golden")
            .join(file),
    )
    .unwrap();
    let devs = deviations();
    let tag = match rev {
        Revision::Vhdl1993 | Revision::Vhdl2008 => "2008",
        Revision::Vhdl2019 => "2019",
    };
    let mut fails = Vec::new();
    let mut n = 0;
    for line in data.split(|&c| c == b'\n').filter(|l| !l.is_empty()) {
        // The result is everything after the last argument; it may itself
        // contain tabs, so split on the "\t[" / "\tERROR" marker.
        let (head, golden): (&[u8], Option<Vec<u8>>) = if line.ends_with(b"\tERROR") {
            (&line[..line.len() - 6], None)
        } else {
            let i = line.windows(2).position(|w| w == b"\t[").unwrap();
            (&line[..i], Some(line[i + 2..line.len() - 1].to_vec()))
        };
        let head = show(head);
        let parts: Vec<&str> = head.split('\t').collect();
        let (func, args) = (parts[0], &parts[1..]);
        if func.starts_with("tool_") {
            continue;
        }
        n += 1;
        let key_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let dev = devs
            .get(&(tag.to_string(), func.to_string(), key_args.clone()))
            .or_else(|| devs.get(&("*".to_string(), func.to_string(), key_args.clone())));
        let mut expected: Option<Vec<u8>> = match dev {
            Some(d) if d == "ERROR" => None,
            Some(d) => Some(unescape(&d[1..d.len() - 1])),
            None => golden.clone(),
        };
        let is_read = func.starts_with("read_") || matches!(func, "hread" | "oread" | "bread");
        if dev.is_none() && is_read && expected.as_ref().is_some_and(|g| g.starts_with(b"g=false"))
        {
            expected = Some(cat(&[b"g=false rest=", &unescape(args[0])]));
        }
        let got = eval(rev, func, args);
        let ok = match (&expected, &got) {
            (Some(x), Ok(g)) => x == g,
            (None, Err(_)) => true,
            _ => false,
        };
        if !ok {
            fails.push(format!(
                "{tag}\t{}\t{}\t{}",
                head,
                match &got {
                    Ok(g) => format!("[{}]", show(g)),
                    Err(e) => format!("ERROR ({e})"),
                },
                match &expected {
                    Some(x) => format!("expected [{}]", show(x)),
                    None => "expected ERROR".into(),
                },
            ));
        }
    }
    assert!(
        fails.is_empty(),
        "{} of {n} golden cases differ:\n{}",
        fails.len(),
        fails.join("\n")
    );
    n
}

#[test]
fn sim1_2008() {
    assert!(run(Revision::Vhdl2008, "sim1-2008.txt") > 2500);
}

#[test]
fn sim1_2019() {
    assert!(run(Revision::Vhdl2019, "sim1-2019.txt") > 2500);
}
