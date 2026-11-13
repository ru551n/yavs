-- difftest: top=inout_record_lanes std=2008 ignore=stderr
-- One AXI-stream style record per interface with `tready` inside and port mode `inout`: each
-- stage drives the fields it owns and 'Z' on the others (two sources per field, resolved).
-- Forty register stages through three levels: a bank with array-of-records ports on slices
-- of one signal (`ch(0 to 39)`, `ch(1 to 40)`), a wrapper per element (`s(s'low + i)`) and the
-- stage inside it; a tap on one element that drives 'Z' on every field (a third source); a
-- second chain of two stages on elements `c2(i)`; and a lane whose tap leaves `tlast`
-- undriven, so its 'U' default is a source of the actual. Each stage runs 1200 activations,
-- past the switch to its specialised process copy, and checks 'event and 'last_value of what
-- it reads.
-- ignore=stderr: the reference simulator warns about the undriven `tlast` of the tap (always
-- 'U'), which is the point of that lane; the warning is left out of the `.expected`.
library ieee;
use ieee.std_logic_1164.all;

package inout_record_lanes_pkg is
  type axs_t is record
    tvalid : std_logic;
    tdata  : std_logic_vector;
    tlast  : std_logic;
    tready : std_logic;
  end record;
  type axs_arr_t is array (natural range <>) of axs_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.inout_record_lanes_pkg.all;

entity inout_record_lanes_stage is
  port (clk : in std_logic; s : inout axs_t; m : inout axs_t);
end;
architecture t of inout_record_lanes_stage is
  signal v, l : std_logic := '0';
  signal d : std_logic_vector(s.tdata'range) := (others => '0');
begin
  s.tready <= '1' when v = '0' or m.tready = '1' else '0';
  process (clk)
  begin
    if rising_edge(clk) then
      if s.tdata'event then
        assert false report "data changed with the clock" severity failure;
      end if;
      if v = '0' or m.tready = '1' then
        v <= s.tvalid;
        d <= std_logic_vector(unsigned(s.tdata) + 1);
        l <= s.tlast;
      end if;
    end if;
  end process;
  m.tvalid <= v;
  m.tdata <= d;
  m.tlast <= l;
  s.tvalid <= 'Z';
  s.tdata <= (s.tdata'range => 'Z');
  s.tlast <= 'Z';
  m.tready <= 'Z';
  mon : process (s.tdata)
    variable seen : std_logic_vector(s.tdata'range) := (others => 'U');
  begin
    if now > 0 ns then
      assert s.tdata'event report "woken without an event" severity failure;
      assert s.tdata'last_value = seen report "wrong 'last_value" severity failure;
    end if;
    seen := s.tdata;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.inout_record_lanes_pkg.all;

entity inout_record_lanes_wrap is
  port (clk : in std_logic; s : inout axs_t; m : inout axs_t);
end;
architecture t of inout_record_lanes_wrap is
begin
  u : entity work.inout_record_lanes_stage port map (clk, s, m);
end;

library ieee;
use ieee.std_logic_1164.all;
use work.inout_record_lanes_pkg.all;

entity inout_record_lanes_bank is
  port (clk : in std_logic; s : inout axs_arr_t; m : inout axs_arr_t);
end;
architecture t of inout_record_lanes_bank is
begin
  g : for i in 0 to s'length - 1 generate
    w : entity work.inout_record_lanes_wrap port map (clk, s(s'low + i), m(m'low + i));
  end generate;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.inout_record_lanes_pkg.all;

-- Drives 'Z' on every field, or leaves `tlast` undriven.
entity inout_record_lanes_tap is
  generic (DRIVE_LAST : boolean);
  port (t : inout axs_t);
end;
architecture t of inout_record_lanes_tap is
begin
  t.tvalid <= 'Z';
  t.tdata <= (t.tdata'range => 'Z');
  t.tready <= 'Z';
  g : if DRIVE_LAST generate
    t.tlast <= 'Z';
  end generate;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.inout_record_lanes_pkg.all;

entity inout_record_lanes is end;
architecture t of inout_record_lanes is
  constant N : positive := 40;
  signal clk : std_logic := '0';
  signal ch : axs_arr_t(0 to N)(tdata(31 downto 0));
  signal c2 : axs_arr_t(0 to 2)(tdata(15 downto 0));
  signal cu : axs_arr_t(0 to 1)(tdata(7 downto 0));
begin
  b : entity work.inout_record_lanes_bank port map (clk, ch(0 to N - 1), ch(1 to N));
  tap : entity work.inout_record_lanes_tap generic map (true) port map (ch(17));
  g2 : for i in 0 to 1 generate
    u : entity work.inout_record_lanes_stage port map (clk, c2(i), c2(i + 1));
  end generate;
  uu : entity work.inout_record_lanes_stage port map (clk, cu(0), cu(1));
  tapu : entity work.inout_record_lanes_tap generic map (false) port map (cu(1));

  process
    variable sum : unsigned(31 downto 0) := (others => '0');
    variable x : unsigned(31 downto 0) := x"0badf00d";
    variable got : natural := 0;
  begin
    ch(0).tready <= 'Z';
    c2(0).tready <= 'Z';
    cu(0).tready <= 'Z';
    for k in 1 to 1200 loop
      x := (x(30 downto 0) & (x(31) xor x(21) xor x(1) xor x(0)));
      if ch(0).tvalid /= '1' or ch(0).tready = '1' then
        ch(0).tvalid <= x(3);
        ch(0).tdata <= std_logic_vector(x);
        ch(0).tlast <= x(5);
      end if;
      ch(N).tready <= x(7) or x(9);
      c2(0).tvalid <= x(4);
      c2(0).tdata <= std_logic_vector(x(15 downto 0));
      c2(0).tlast <= x(6);
      c2(2).tready <= x(8);
      cu(0).tvalid <= '1';
      cu(0).tdata <= std_logic_vector(x(7 downto 0));
      cu(0).tlast <= '1';
      cu(1).tready <= '1';
      wait for 1 ns;
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
      wait for 1 ns;
      if ch(N).tvalid = '1' and ch(N).tready = '1' then
        got := got + 1;
        sum := (sum(26 downto 0) & sum(31 downto 27)) xor unsigned(ch(N).tdata);
        if ch(N).tlast = '1' then sum := sum + 2; end if;
      end if;
      if c2(2).tvalid = '1' and c2(2).tready = '1' then
        sum := sum + unsigned(c2(2).tdata);
      end if;
      if k mod 300 = 0 then
        report "k=" & integer'image(k) & " got=" & integer'image(got) & " sum=" & to_hstring(sum)
          & " ch(40)=" & to_hstring(ch(N).tdata) & " ch(17)=" & to_hstring(ch(17).tdata)
          & " last=" & to_hstring(ch(17).tdata'last_value) & " ready=" & to_string(ch(17).tready)
          & " c2(2)=" & to_hstring(c2(2).tdata) & " cu(1).tlast=" & to_string(cu(1).tlast)
          & " cu(1).tvalid=" & to_string(cu(1).tvalid);
      end if;
    end loop;
    assert cu(1).tlast = 'U' report "undriven inout field is not a 'U' source" severity failure;
    assert got > 100 report "too few transfers" severity failure;
    wait;
  end process;
end;
