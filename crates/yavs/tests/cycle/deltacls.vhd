-- Clocks whose edges come in a known delta of their time step: made by assignments with a delay
-- (the first delta) and by clock processes that wait for a time (the second), with logic between
-- registers of such clocks at coincident edges, against the normal mode. Logic from registers of
-- the first kind to registers of the second stays out: those sample it before it settles.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_deltacls is
end entity;
architecture t of cycle_deltacls is
  signal ca, cb, cc : std_ulogic := '0';
  signal cd : std_ulogic := '1';
  signal v : std_ulogic_vector(0 to 1) := "00";
  signal ra, rb, rc, rd, re, rv0, rv1, rx : unsigned(W - 1 downto 0) := (others => '0');
  signal xa, xb, xc, xd, xv : unsigned(W - 1 downto 0);
  signal stop : boolean := false;
begin
  -- The first delta: two assignments with a delay, and both elements of one vector at once.
  ca <= not ca after 5 ns when not stop;
  cb <= not cb after 7.5 ns when not stop;
  v <= (others => not v(0)) after 7.5 ns when not stop;
  -- The second: two clock processes waiting for times elaboration knows.
  pc : process
  begin
    wait for 5 ns;
    if stop then
      wait;
    end if;
    cc <= not cc;
  end process;
  pd : process
    constant half : time := 2.5 ns;
  begin
    wait for half;
    if stop then
      wait;
    end if;
    cd <= not cd;
  end process;

  ga : process (ca) begin if rising_edge(ca) then ra <= ra + 1 + xb; end if; end process;
  gb : process (cb) begin if rising_edge(cb) then rb <= rb + xa; end if; end process;
  gv0 : process (v(0)) begin if rising_edge(v(0)) then rv0 <= rv0 + 3 + xv; end if; end process;
  gv1 : process (v(1)) begin if rising_edge(v(1)) then rv1 <= rv1 + 1; end if; end process;
  gc : process (cc) begin if rising_edge(cc) then rc <= rc + 1 + xd; end if; end process;
  gd : process (cd) begin if rising_edge(cd) then rd <= rd + 5 + xc; end if; end process;
  -- A register of the first kind sampling logic of the second: in.
  ge : process (ca) begin if rising_edge(ca) then re <= re + xc; end if; end process;
  -- A register of the second kind sampling logic of the first: out.
  gx : process (cc) begin if rising_edge(cc) then rx <= rx + xa; end if; end process;
  xa <= ra + 7;
  xb <= rb(W - 1 downto 1) & '1';
  xv <= rv1 + 2;
  xc <= rc + 3;
  xd <= rd xor rc;

  main : process
  begin
    wait for 2.5 ns;
    for k in 1 to 24 loop
      wait for 5 ns;
      report "k=" & integer'image(k) & " " & to_hstring(ra) & " " & to_hstring(rb) & " "
        & to_hstring(rv0) & " " & to_hstring(rc) & " " & to_hstring(rd) & " "
        & to_hstring(re) & " " & to_hstring(rx);
    end loop;
    stop <= true;
    wait;
  end process;
end architecture;

-- A clock process waiting 0 ns every other time: its rising edges come a delta after those of
-- the other clock process, so its timeout, which elaboration does not know, keeps it apart and
-- the logic between them in the event kernel.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_deltazero is
end entity;
architecture t of cycle_deltazero is
  signal cc : std_ulogic := '0';
  signal cz : std_ulogic := '1';
  signal rc, rz : unsigned(7 downto 0) := (others => '0');
  signal xc, xz : unsigned(7 downto 0);
begin
  pc : process
  begin
    wait for 5 ns;
    if now > 40 ns then
      wait;
    end if;
    cc <= not cc;
  end process;
  pz : process
    variable half : time := 5 ns;
  begin
    wait for half;
    half := 5 ns - half;
    if now > 40 ns then
      wait;
    end if;
    cz <= not cz;
  end process;
  gc : process (cc) begin if rising_edge(cc) then rc <= rc + 1 + xz; end if; end process;
  gz : process (cz) begin if rising_edge(cz) then rz <= rz + xc; end if; end process;
  xc <= rc + 3;
  xz <= rz + 1;
  main : process
  begin
    wait for 47 ns;
    report "rc=" & to_hstring(rc) & " rz=" & to_hstring(rz);
    wait;
  end process;
end architecture;
