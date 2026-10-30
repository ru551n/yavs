//! Resolution functions use the normal native ABI and their declaring instance's data.
mod common;
use std::process::Command;

fn check(name: &str, source: &str, standards: &[&str]) {
    for std in standards {
        let dir = common::tempdir(name);
        std::fs::write(dir.join("t.vhd"), source).expect("write");
        let run = |args: &[&str]| {
            let out = Command::new(env!("CARGO_BIN_EXE_yavs"))
                .current_dir(&dir)
                .args(["--color", "never"])
                .args(args)
                .output()
                .expect("run");
            assert!(
                out.status.success(),
                "std={std}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["analyze", "--std", std, "--work=work:lib", "t.vhd"]);
        for flags in [
            &["--no-cache"][..],
            &[][..],
            &[][..],
            &["--threads=4"][..],
            &["--cov=a.ycov"][..],
        ] {
            let mut args = vec!["run", "--std", std, "--work=work:lib", "--tier=interp"];
            args.extend(flags);
            args.push("top");
            run(&args);
        }
    }
}

#[test]
fn predefined_functions_can_resolve_signals_through_aliases() {
    check(
        "native-resolvers",
        r#"
      package p is
        alias max_res is maximum [integer_vector return integer];
        alias min_res is minimum [integer_vector return integer];
        alias real_res is maximum [real_vector return real];
        alias or_res is "or" [bit_vector return bit];
        subtype high_int is max_res integer;
        subtype low_int is min_res integer;
        subtype high_real is real_res real;
        subtype or_bit is or_res bit;
      end;
      use work.p.all;
      entity top is end;
      architecture a of top is
        signal hi : high_int;
        signal lo : low_int;
        signal x : high_real;
        signal b : or_bit;
      begin
        hi <= 3; hi <= 5;
        lo <= 3; lo <= 5;
        x <= 1.25; x <= 2.5;
        b <= '0'; b <= '1';
        process begin
          wait for 1 ns;
          assert hi = 5 and lo = 3 and x = 2.5 and b = '1'
            report "native resolver result" severity failure;
          wait;
        end process;
      end;
    "#,
        &["2008"],
    );
}

#[test]
fn block_and_generate_resolvers_capture_their_declaring_region() {
    check(
        "block-resolvers",
        r#"
      entity top is generic (adjustment : integer := 10); end;
      architecture a of top is begin
        g : for i in 0 to 1 generate
          b : block
            type iv is array (natural range <>) of integer;
            constant bias : integer := adjustment + i;
            function resolve(v : iv) return integer is
              variable total : integer := bias;
            begin
              for k in v'range loop total := total + v(k); end loop;
              return total;
            end;
            subtype resolved_int is resolve integer;
          begin
            inner_block : block
              signal s : resolved_int := 0;
            begin
              s <= 3; s <= 5;
              process begin
                wait for 1 ns;
                assert s = 8 + adjustment + i report "wrong resolver region" severity failure;
                wait;
              end process;
            end block;
          end block;
        end generate;
      end;
    "#,
        &["93", "2008"],
    );
}

#[test]
fn a_resolver_declared_in_a_generate_uses_each_iterations_parameter() {
    check(
        "generate-resolvers",
        r#"
      entity top is end;
      architecture a of top is
        type iv is array (natural range <>) of integer;
      begin
        g : for i in 0 to 2 generate
          function resolve(v : iv) return integer is
            variable total : integer := i;
          begin
            for k in v'range loop total := total + v(k); end loop;
            return total;
          end;
          subtype ri is resolve integer;
          signal s : ri := 0;
        begin
          s <= 3; s <= 5;
          process begin
            wait for 1 ns;
            assert s = 8 + i report "wrong generate resolver region" severity failure;
            wait;
          end process;
        end generate;
      end;
    "#,
        &["93", "2008"],
    );
}
