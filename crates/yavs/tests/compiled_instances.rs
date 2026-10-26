//! Instances of one process share the addresses of their own objects (instance data, heap
//! objects at the same indices): the compiled tier's object table keys them by machine as well
//! as address (#679), so each instance must see only its own objects after the others ran, also
//! after a deallocation changed its heap. Every tier gives the interpreter's output.
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"entity cell is
  generic (seed : integer);
  port (clk : in bit; sum : out integer);
end;
architecture a of cell is
  type ints_t is array (natural range <>) of integer;
  type ints_ptr_t is access ints_t;
  constant tab : ints_t(0 to 7) := (3, 1, 4, 1, 5, 9, 2, 6);
begin
  process (clk)
    variable buf : ints_ptr_t := new ints_t'(0 to 7 => 0);
    variable aux : ints_ptr_t := new ints_t'(0 to 3 => 0);
    variable n   : integer := 0;
    variable acc : integer := seed;
  begin
    if clk = '1' then
      n := n + 1;
      buf(n mod 8) := (buf((n + 3) mod 8) + seed * tab(n mod 8) + n) mod 100003;
      aux(n mod 4) := (aux(n mod 4) + buf(n mod 8)) mod 1009;
      -- Every few edges the heap changes: the old objects go, new ones take their slots.
      if n mod (3 + seed) = 0 then
        deallocate(aux);
        aux := new ints_t'(seed, n, seed + n, 7);
      end if;
      acc := (acc + buf(n mod 8) + aux(n mod 4)) mod 1000003;
      sum <= acc;
    end if;
  end process;
end;

entity instances is
end;
architecture a of instances is
  type sums_t is array (1 to 4) of integer;
  signal sums : sums_t;
  signal clk : bit := '0';
begin
  cells : for k in 1 to 4 generate
    u : entity work.cell generic map (seed => k) port map (clk => clk, sum => sums(k));
  end generate;
  process
  begin
    for i in 1 to 200 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    for k in sums'range loop
      report "sum " & integer'image(k) & " " & integer'image(sums(k));
    end loop;
    wait;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

#[test]
fn instances_keep_their_own_objects_in_compiled_code() {
    let dir = tempdir("compiled_instances");
    std::fs::write(dir.join("instances.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "instances.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    let run = |flags: &[&str]| {
        let mut args = vec!["run", "--std=2008", "--work=work:work"];
        args.extend_from_slice(flags);
        args.push("instances");
        let o = yavs(&dir, &args);
        (
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            o.status.code(),
        )
    };
    let want = run(&["--tier=interp"]);
    assert_eq!(want.2, Some(0), "{want:?}");
    assert_eq!(want.0.matches("sum ").count(), 4, "{want:?}");
    for flags in [
        vec!["--tier=llvm"],
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift-auto", "--tier-heat=1,0,0", "--threads=4"],
        #[cfg(feature = "clift")]
        vec!["--tier=auto3", "--tier-heat=1,0,0"],
    ] {
        assert_eq!(run(&flags), want, "{flags:?}");
    }
}
