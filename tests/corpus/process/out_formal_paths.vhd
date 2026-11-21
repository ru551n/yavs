-- difftest: top=out_formal_paths std=2008 ref=hand
-- A scalar out formal assigned whole on every path before any read (if/elsif/else, case, a
-- conditional assignment with a final else, after statements that do not name it) ends with
-- the value assigned; one assigned on some paths only, read first on one path, named in a loop,
-- passed on, or left by `return` or `wait` first starts at its subtype's 'left (LRM 4.2.2.2, 14.6 b) 5),
-- #1184): by reference, through a narrower actual's temporary and through a logic element.
-- sim1 starts an out formal at the actual's value instead; the .expected is hand-checked
-- against the LRM (sim2 agrees).
library ieee;
use ieee.std_logic_1164.all;
entity out_formal_paths is end entity;
architecture a of out_formal_paths is
  subtype s is integer range 3 to 9;
  procedure both (x : out integer; v : integer) is
  begin
    if v > 0 then x := v; else x := 0; end if;
  end procedure;
  procedure chain (x : out integer; v : integer) is
  begin
    report "chain v=" & integer'image(v);
    assert v < 100 report "big";
    if v > 10 then x := 2; elsif v > 0 then x := 1; else x := 0; end if;
  end procedure;
  procedure sel (x : out integer; v : integer) is
  begin
    case v is
      when 0 => x := 10;
      when 1 | 2 => if v = 1 then x := 11; else x := 12; end if;
      when others => null; x := 13;
    end case;
  end procedure;
  procedure cond (x : out integer; v : integer) is
  begin
    x := 1 when v > 0 else 2;
  end procedure;
  procedure half (x : out integer; v : integer) is
  begin
    if v > 0 then x := v; end if;
  end procedure;
  procedure half_s (x : out s; v : integer) is
  begin
    if v > 0 then x := v; end if;
  end procedure;
  procedure half_case (x : out integer; v : integer) is
  begin
    case v is
      when 0 => x := 10;
      when others => null;
    end case;
  end procedure;
  procedure cond_half (x : out integer; v : integer) is
  begin
    x := 1 when v > 0;
  end procedure;
  procedure rd_branch (x : out integer; v : integer) is
  begin
    if v > 0 then report "rd x=" & integer'image(x); x := v; else x := 0; end if;
  end procedure;
  procedure rd_cond (x : out s) is
  begin
    if x = 3 then x := 4; else x := 5; end if;
  end procedure;
  procedure rd_late (x : out integer; v : integer) is
  begin
    if v > 0 then x := v; end if;
    report "late x=" & integer'image(x);
    x := 7;
  end procedure;
  procedure early (x : out integer; v : integer) is
  begin
    if v > 0 then return; end if;
    x := 1;
  end procedure;
  procedure looped (x : out integer; v : integer) is
  begin
    for i in 1 to v loop
      x := i;
    end loop;
  end procedure;
  procedure ret_loop (x : out integer; v : integer) is
  begin
    for i in 1 to 3 loop
      if i = v then return; end if;
    end loop;
    x := 1;
  end procedure;
  procedure waits (x : out integer; v : integer) is
  begin
    wait for 0 ns;
    if v > 0 then x := v; else x := 0; end if;
  end procedure;
  procedure passed (x : out integer; v : integer) is
  begin
    half(x, v);
  end procedure;
  procedure both_l (x : out std_logic; v : integer) is
  begin
    if v > 0 then x := '1'; else x := '0'; end if;
  end procedure;
  procedure half_l (x : out std_logic; v : integer) is
  begin
    if v > 0 then x := '1'; end if;
  end procedure;
begin
  process
    variable v : integer;
    variable w : integer range 0 to 100;
    variable lv : std_logic_vector(0 to 3) := "1111";
    procedure show (what : string; x : integer) is
    begin
      report what & "=" & integer'image(x);
    end procedure;
  begin
    v := 42; both(v, 3); show("both", v);
    v := 42; both(v, -1); show("both", v);
    v := 42; chain(v, 5); show("chain", v);
    v := 42; sel(v, 0); show("sel", v);
    v := 42; sel(v, 2); show("sel", v);
    v := 42; sel(v, 7); show("sel", v);
    v := 42; cond(v, 1); show("cond", v);
    v := 42; cond(v, 0); show("cond", v);
    v := 42; half(v, 1); show("half", v);
    v := 42; half(v, 0); show("half", v);
    w := 50; both(w, 9); show("both w", w);
    w := 50; half_s(w, 0); show("half_s w", w);
    v := 42; half_case(v, 1); show("half_case", v);
    v := 42; cond_half(v, 0); show("cond_half", v);
    v := 42; rd_branch(v, 1); show("rd_branch", v);
    v := 42; rd_cond(v); show("rd_cond", v);
    v := 42; rd_late(v, 0); show("rd_late", v);
    v := 42; early(v, 1); show("early", v);
    v := 42; early(v, 0); show("early", v);
    v := 42; looped(v, 0); show("looped", v);
    v := 42; looped(v, 2); show("looped", v);
    v := 42; ret_loop(v, 2); show("ret_loop", v);
    v := 42; ret_loop(v, 5); show("ret_loop", v);
    v := 42; passed(v, 0); show("passed", v);
    v := 42; waits(v, 4); show("waits", v);
    both_l(lv(1), 0);
    half_l(lv(2), 0);
    report "lv=" & std_logic'image(lv(1)) & std_logic'image(lv(2));
    wait;
  end process;
end architecture;
