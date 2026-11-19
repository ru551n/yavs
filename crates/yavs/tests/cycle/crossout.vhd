-- Logic between registers of clocks whose edges come in different deltas, on paths through
-- processes outside the region: one whose history the testbench reads, one assigning a
-- metavalue literal the testbench sees (--cycle-exact-meta), a register outside as the source,
-- a converted port. The clock cg2 is ca gated twice in the kernel, so its edges come two deltas
-- after those of ca. The logic stays out; logic between registers of ca alone joins.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xout_sub is
  generic (W : positive);
  port (d : in std_logic_vector(W - 1 downto 0); q : out unsigned(W - 1 downto 0));
end entity;
architecture t of cycle_xout_sub is
begin
  q <= unsigned(d) + 1;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xout is
  generic (W : positive := 8);
end entity;
architecture t of cycle_xout is
  signal ca : std_ulogic := '0';
  signal en : std_ulogic := '1';
  signal cg1, cg2 : std_ulogic;
  signal ra, rb, rx, ry, rz, rp, rs : unsigned(W - 1 downto 0) := (others => '0');
  signal xa, y, z, xb, yb, m, xm, xs, xs2, xq, xp : unsigned(W - 1 downto 0);
  signal stop : boolean := false;
begin
  ca <= not ca after 5 ns when not stop;
  cg1 <= ca and en;
  cg2 <= cg1 and en;
  ga : process (ca) begin if rising_edge(ca) then ra <= ra + 1; end if; end process;
  -- Through a process whose history the testbench reads, then a member.
  xa <= ra + 7;
  y <= xa + 2;
  z <= y + 1;
  gx : process (cg2) begin if rising_edge(cg2) then rx <= rx + y + z; end if; end process;
  -- The same path between registers of ca: it joins.
  xb <= ra xor rb;
  yb <= xb + 3;
  gb : process (ca) begin if rising_edge(ca) then rb <= rb + yb + 1; end if; end process;
  -- Through a process assigning a metavalue literal the testbench sees.
  xm <= ra + 5;
  m <= xm when en = '1' else (others => 'X');
  gy : process (cg2) begin if rising_edge(cg2) then ry <= ry + m; end if; end process;
  -- From a register outside (its history is read).
  gs : process (ca) begin if rising_edge(ca) then rs <= rs + 3; end if; end process;
  xs <= rs + 1;
  xs2 <= xs xor rs;
  gz : process (cg2) begin if rising_edge(cg2) then rz <= rz + xs2 + 1; end if; end process;
  -- Through a converted port.
  xq <= ra + 9;
  u : entity work.cycle_xout_sub generic map (W) port map (d => std_logic_vector(xq), q => xp);
  gp : process (cg2) begin if rising_edge(cg2) then rp <= rp + xp; end if; end process;
  main : process
  begin
    for k in 1 to 10 loop
      wait for 10 ns;
      report "k=" & integer'image(k) & " " & to_hstring(rx) & " " & to_hstring(rb) & " "
        & to_hstring(ry) & " " & to_hstring(rz) & " " & to_hstring(rp) & " " & to_hstring(m)
        & " " & time'image(y'last_event) & " " & time'image(rs'last_event) & " "
        & time'image(yb'last_event);
    end loop;
    stop <= true;
    wait;
  end process;
end architecture;

-- A clock gated twice by members and once more in the kernel: its edges would come a delta
-- early. The gate reading the clock from outside joins; the one reading the member's gate stays
-- out.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xgate is
  generic (W : positive := 8);
end entity;
architecture t of cycle_xgate is
  signal ca : std_ulogic := '0';
  signal en : std_ulogic := '1';
  signal cg0, cg1, cg2 : std_ulogic;
  signal ra, rx : unsigned(W - 1 downto 0) := (others => '0');
  signal y1, y2 : unsigned(W - 1 downto 0);
  signal stop : boolean := false;
begin
  ca <= not ca after 5 ns when not stop;
  cg0 <= ca and en;
  cg1 <= cg0 and en;
  cg2 <= cg1 and en;
  ga : process (ca) begin if rising_edge(ca) then ra <= ra + 1; end if; end process;
  y1 <= ra + 7;
  y2 <= y1 + 1;
  gx : process (cg2) begin if rising_edge(cg2) then rx <= rx + y2; end if; end process;
  main : process
  begin
    for k in 1 to 10 loop
      wait for 10 ns;
      report "k=" & integer'image(k) & " " & to_hstring(rx) & " " & time'image(y1'last_event)
        & " " & time'image(y2'last_event);
    end loop;
    stop <= true;
    wait;
  end process;
end architecture;

-- Processes of the testbench kind on the late clock: a clocked logger writing with text I/O and a
-- clocked checker of the top-level unit that also reads a testbench signal (it stays in the event
-- kernel). Both sample logic of registers of ca at their edge: that logic stays out.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;
entity cycle_xsink is
  generic (W : positive := 8);
end entity;
architecture t of cycle_xsink is
  signal ca : std_ulogic := '0';
  signal en, tbs : std_ulogic := '1';
  signal cg1, cg2 : std_ulogic;
  signal ra : unsigned(W - 1 downto 0) := (others => '0');
  signal xl1, xl2, xc1, xc2 : unsigned(W - 1 downto 0);
  signal stop : boolean := false;
begin
  ca <= not ca after 5 ns when not stop;
  cg1 <= ca and en;
  cg2 <= cg1 and en;
  ga : process (ca) begin if rising_edge(ca) then ra <= ra + 1; end if; end process;
  xl1 <= ra xor (ra srl 1);
  xl2 <= xl1 + 1;
  xc1 <= not ra;
  xc2 <= xc1 xor (ra sll 1);
  logger : process (cg2)
    variable l : line;
  begin
    if rising_edge(cg2) then
      write(l, string'("lg=") & to_hstring(xl2));
      writeline(output, l);
    end if;
  end process;
  checker : process (cg2)
  begin
    if rising_edge(cg2) then
      report "ck=" & to_hstring(xc2) & " " & std_ulogic'image(tbs);
    end if;
  end process;
  main : process
  begin
    wait for 100 ns;
    tbs <= '0';
    stop <= true;
    wait;
  end process;
end architecture;
