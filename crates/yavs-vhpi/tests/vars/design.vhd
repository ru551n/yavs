-- Process and shared variables read and deposited through VHPI (#309): scalars, a logic
-- vector, a string and a record, by the `vars` test plugin (tests/plugin/vars.c). Analysed
-- with `--relaxed` (shared variables of non-protected types).
library ieee;
use ieee.std_logic_1164.all;

package pkg is
  shared variable pv : std_logic_vector(3 downto 0) := "1010";
  type counter_t is protected
    procedure bump;
  end protected;
end package;

package body pkg is
  type counter_t is protected body
    variable n : integer := 0;
    procedure bump is
    begin
      n := n + 1;
    end procedure;
  end protected body;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity top is
end entity;

architecture a of top is
  type rec_t is record
    a : integer;
    b : std_logic_vector(3 downto 0);
    t : time;
  end record;
  signal clk : std_logic := '0';
  shared variable sv : integer := 5;
  shared variable pc : work.pkg.counter_t;
  shared variable un : integer := 7;
begin
  clk <= not clk after 5 ns when now < 40 ns;

  p : process (clk)
    variable n : integer := 0;
    variable v : std_logic_vector(7 downto 0) := x"00";
    variable r : rec_t := (1, "0000", 2 ns);
    variable b : boolean := false;
    variable s : string(1 to 3) := "abc";
    variable x : real := 0.5;
  begin
    if rising_edge(clk) then
      n := n + 1;
      v := std_logic_vector(unsigned(v) + 1);
      r.a := r.a * 2;
      report "n=" & integer'image(n) & " v=" & to_string(v) & " r.a=" & integer'image(r.a)
        & " r.b=" & to_string(r.b) & " r.t=" & time'image(r.t) & " b=" & boolean'image(b)
        & " s=" & s;
    end if;
  end process;

  -- Shared variables: an architecture's and a package's.
  sh : process (clk)
  begin
    if rising_edge(clk) then
      sv := sv + 1;
      report "sv=" & integer'image(sv) & " pv=" & to_string(work.pkg.pv);
    end if;
  end process;

  -- Deposited before it first runs (#342): the first run sees the deposited value.
  d : process
    variable m : integer := 1;
  begin
    report "m=" & integer'image(m);
    wait;
  end process;

  -- Suspended inside a procedure: its variable is still the process's.
  q : process
    variable k : natural := 3;
    procedure tick is
    begin
      wait for 10 ns;
    end procedure;
  begin
    tick;
    report "k=" & integer'image(k);
    k := k + 1;
  end process;
end architecture;
