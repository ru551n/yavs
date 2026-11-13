-- difftest: top=inout_record_whole std=2008
-- Whole-record drives of `inout` record ports, AXI-stream style with `tready` inside: each
-- stage assigns its whole `m` record (data fields, 'Z' on `tready`) and its whole `s` record
-- ('Z' on the data fields, its ready on `tready`) in one statement each. Two shapes: lanes on
-- elements `ch(i)` of an array of records, and lanes on plain narrow record signals `r0`..`r3`
-- (packs of leaves) that are initialised to '0' while the ports default to all 'Z' (the
-- drivers start at the port's default). Each stage runs 400 activations and checks what it
-- reads.
library ieee;
use ieee.std_logic_1164.all;

package iorw_pkg is
  type axs_t is record
    tvalid : std_logic;
    tdata  : std_logic_vector(7 downto 0);
    tlast  : std_logic;
    tready : std_logic;
  end record;
  constant AXS_Z : axs_t := (tvalid => 'Z', tdata => (others => 'Z'), tlast => 'Z', tready => 'Z');
  constant AXS_0 : axs_t := (tvalid => '0', tdata => (others => '0'), tlast => '0', tready => '0');
  type axs_arr_t is array (natural range <>) of axs_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.iorw_pkg.all;

entity iorw_stage is
  port (clk : in std_logic; s : inout axs_t := AXS_Z; m : inout axs_t := AXS_Z);
end;
architecture t of iorw_stage is
  signal v, l : std_logic := '0';
  signal d : std_logic_vector(7 downto 0) := x"00";
begin
  m <= (tvalid => v, tdata => d, tlast => l, tready => 'Z');
  s <= (tvalid => 'Z', tdata => (others => 'Z'), tlast => 'Z', tready => not v or m.tready);
  process (clk)
  begin
    if rising_edge(clk) then
      if v = '0' or m.tready = '1' then
        v <= s.tvalid;
        d <= std_logic_vector(unsigned(s.tdata) + 5);
        l <= s.tlast;
      end if;
    end if;
  end process;
  mon : process (s)
  begin
    if now > 0 ns and s.tvalid = '1' then
      assert s.tdata /= "ZZZZZZZZ" report "data not driven" severity failure;
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.iorw_pkg.all;

entity inout_record_whole is end;
architecture t of inout_record_whole is
  signal clk : std_logic := '0';
  signal ch : axs_arr_t(0 to 3);
  signal r0, r1, r2, r3 : axs_t := AXS_0;
begin
  g : for i in 0 to 2 generate
    u : entity work.iorw_stage port map (clk, ch(i), ch(i + 1));
  end generate;
  p0 : entity work.iorw_stage port map (clk, r0, r1);
  p1 : entity work.iorw_stage port map (clk, r1, r2);
  p2 : entity work.iorw_stage port map (clk, r2, r3);

  src : process
    variable x : unsigned(7 downto 0) := x"a5";
    variable sum : unsigned(15 downto 0) := x"0000";
    variable got : natural := 0;
  begin
    report "init ch(1)=" & to_string(ch(1).tvalid) & to_hstring(ch(1).tdata) & to_string(ch(1).tready)
      & " r1=" & to_string(r1.tvalid) & to_hstring(r1.tdata) & to_string(r1.tready);
    for k in 1 to 400 loop
      x := x(6 downto 0) & (x(7) xor x(5) xor x(4) xor x(3));
      if ch(0).tvalid /= '1' or ch(0).tready = '1' then
        ch(0) <= (tvalid => x(0), tdata => std_logic_vector(x), tlast => x(1), tready => 'Z');
      end if;
      ch(3) <= (tvalid => 'Z', tdata => (others => 'Z'), tlast => 'Z', tready => x(2) or x(3));
      if r0.tvalid /= '1' or r0.tready = '1' then
        r0 <= (tvalid => x(4), tdata => std_logic_vector(x + 17), tlast => x(5), tready => 'Z');
      end if;
      r3 <= (tvalid => 'Z', tdata => (others => 'Z'), tlast => 'Z', tready => x(6));
      wait for 1 ns;
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
      wait for 1 ns;
      if ch(3).tvalid = '1' and ch(3).tready = '1' then
        got := got + 1;
        sum := (sum rol 3) xor resize(unsigned(ch(3).tdata), 16);
      end if;
      if r3.tvalid = '1' and r3.tready = '1' then
        sum := sum + unsigned(r3.tdata);
        if r3.tlast = '1' then sum := sum + 1; end if;
      end if;
      if k mod 100 = 0 then
        report "k=" & integer'image(k) & " got=" & integer'image(got) & " sum=" & to_hstring(sum)
          & " ch(2)=" & to_hstring(ch(2).tdata) & to_string(ch(2).tready) & " r2="
          & to_string(r2.tvalid) & to_hstring(r2.tdata) & to_string(r2.tlast) & to_string(r2.tready);
      end if;
    end loop;
    assert got > 50 report "too few transfers" severity failure;
    wait;
  end process;
end;
