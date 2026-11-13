-- difftest: top=real_negative_zero std=2008 ref=hand
-- A change of a REAL signal from 0.0 to -0.0 (or back) is a transaction but no event: the
-- values are equal (LRM 9.2.3), so nothing is woken and 'last_value stays (LRM 14.7.3.2).
-- The first reference simulator makes an event; the second one and this test do not (#1145).
entity counter is
  port (p : in real; n : out integer);
end;
architecture a of counter is
  signal c : integer := 0;
begin
  count : process (p) begin c <= c + 1; end process;
  n <= c;
end;

library ieee;
use ieee.std_logic_1164.all;
entity real_negative_zero is end;

architecture t of real_negative_zero is
  type rec is record r : real; l : std_logic; end record;
  type inner is record x : real; v : real_vector(0 to 1); c : std_logic; end record;
  type outer is record i : inner; y : real; end record;
  type rarr is array (0 to 1) of rec;
  function rf (v : real_vector) return real is begin return v(v'low); end;
  function ident (x : real) return real is begin return x; end;
  subtype rreal is rf real;

  signal s : real := 0.0;
  signal sv : real_vector(0 to 2) := (0.0, 1.0, 0.0);
  signal rc : rec := (0.0, '0');
  signal o : outer := ((0.0, (0.0, 0.0), '0'), 0.0);
  signal ra : rarr := (others => (0.0, '0'));
  signal rr : rreal := 0.0;
  type volt is range -5.0 to 5.0;
  signal dl : real := 0.0;
  signal vt : volt := 0.0;
  -- event counts, each counter runs once at initialisation
  signal n_s, n_sv, n_rc, n_rcr, n_o, n_ox, n_ov, n_ra, n_rr, n_pc, n_pv, n_px, n_dl, n_vt : integer := 0;
begin
  c_s   : process (s)       begin n_s <= n_s + 1; end process;
  c_sv  : process (sv)      begin n_sv <= n_sv + 1; end process;
  c_rc  : process (rc)      begin n_rc <= n_rc + 1; end process;
  c_rcr : process (rc.r)    begin n_rcr <= n_rcr + 1; end process;
  c_o   : process (o)       begin n_o <= n_o + 1; end process;
  c_ox  : process (o.i.x)   begin n_ox <= n_ox + 1; end process;
  c_ov  : process (o.i.v)   begin n_ov <= n_ov + 1; end process;
  c_ra  : process (ra)      begin n_ra <= n_ra + 1; end process;
  c_rr  : process (rr)      begin n_rr <= n_rr + 1; end process;
  c_dl  : process (dl)      begin n_dl <= n_dl + 1; end process;
  c_vt  : process (vt)      begin n_vt <= n_vt + 1; end process;

  -- a port collapsed onto the signal, one over a conversion (an edge of its own), one on a slice
  u_c : entity work.counter port map (p => s, n => n_pc);
  u_x : entity work.counter port map (p => ident(s), n => n_px);
  u_v : entity work.counter port map (p => sv(2), n => n_pv);

  drv : process
    variable z : real := 0.0;
    procedure show (tag : string) is
    begin
      report tag & " n: s=" & integer'image(n_s) & " sv=" & integer'image(n_sv)
        & " rc=" & integer'image(n_rc) & " rc.r=" & integer'image(n_rcr)
        & " o=" & integer'image(n_o) & " o.i.x=" & integer'image(n_ox)
        & " o.i.v=" & integer'image(n_ov) & " ra=" & integer'image(n_ra)
        & " rr=" & integer'image(n_rr) & " port=" & integer'image(n_pc)
        & " conv=" & integer'image(n_px) & " slice=" & integer'image(n_pv)
        & " dl=" & integer'image(n_dl) & " vt=" & integer'image(n_vt);
    end procedure;
  begin
    z := -z;                            -- negative zero
    wait for 1 ns;
    s <= z; sv <= (z, 1.0, z); rc.r <= z; o.i.x <= z; o.i.v(1) <= z; o.y <= z;
    ra(1).r <= z; rr <= z;
    wait for 0 ns;
    report "active=" & boolean'image(s'active) & " " & boolean'image(sv'active)
      & " " & boolean'image(rc.r'active) & " " & boolean'image(o.i.x'active)
      & " " & boolean'image(ra(1).r'active) & " " & boolean'image(rr'active)
      & " event=" & boolean'image(s'event) & " " & boolean'image(sv'event)
      & " " & boolean'image(rc.r'event) & " " & boolean'image(o.i.x'event)
      & " " & boolean'image(ra(1).r'event) & " " & boolean'image(rr'event)
      & " lv=" & boolean'image(s'last_value = 0.0);
    wait for 1 ns;
    show("after -0.0");
    s <= 0.0; sv <= (0.0, 1.0, 0.0); rc.r <= 0.0; o.i.x <= 0.0; o.i.v(1) <= 0.0; o.y <= 0.0;
    ra(1).r <= 0.0; rr <= 0.0;
    wait for 1 ns;
    show("after 0.0 again");
    -- a change of value is an event, and 'last_value is the zero it left
    s <= 1.0; sv(2) <= 2.0; rc.r <= 1.0; o.i.x <= 1.0; o.i.v(1) <= 1.0; ra(1).r <= 1.0; rr <= 1.0;
    wait for 0 ns;
    report "event=" & boolean'image(s'event) & " " & boolean'image(sv'event)
      & " " & boolean'image(rc.r'event) & " " & boolean'image(o.i.x'event)
      & " " & boolean'image(ra(1).r'event) & " " & boolean'image(rr'event)
      & " lv=" & boolean'image(s'last_value = 0.0);
    wait for 1 ns;
    show("after 1.0");
    -- a record whose std_logic field really changes
    rc.l <= '1';
    wait for 1 ns;
    show("after l");
    z := 0.0; z := -z;
    s <= 0.0; rc.r <= 0.0; wait for 1 ns; show("to 0.0");
    -- 'last_value is the 1.0 both zeros followed
    s <= z; rc.r <= z; wait for 0 ns;
    report "to -0.0 event=" & boolean'image(s'event) & " " & boolean'image(rc.r'event)
      & " lv=" & boolean'image(s'last_value = 1.0) & " " & boolean'image(rc.r'last_value = 1.0);
    wait for 1 ns; show("to -0.0");
    -- delayed assignments: after, transport, a user floating type
    dl <= z after 1 ns; vt <= volt(z) after 1 ns; wait for 3 ns; show("delayed -0.0");
    dl <= 1.0 after 1 ns; vt <= 1.0 after 1 ns; wait for 3 ns; show("delayed 1.0");
    dl <= 0.0 after 1 ns; vt <= 0.0 after 1 ns; wait for 3 ns; show("delayed 0.0");
    dl <= transport z after 1 ns; vt <= transport volt(z) after 1 ns; wait for 3 ns;
    show("delayed transport -0.0");
    wait;
  end process;
end;
