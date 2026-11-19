-- Cycle mode: inputs read when they change and settled at the next edge, against inputs settled
-- at once. `a`, `ae` and `b` feed only a register through combinational logic (`x`), so their
-- changes wait for the edge: `ae` changes in the delta of the edge (from the process that makes
-- the clock), `b` between edges, `a` after the edge (from a clocked stimulus process) and twice
-- more between two edges, back to its value. `valid` reaches the output `ready` through combinational logic, and a bus model
-- outside looks at `ready` two deltas after raising `valid`; `sel` reaches the register `tq`
-- through a table look-up. `ext` makes the asynchronous reset `arst` of `r` through combinational
-- logic, and the reset branch copies `rv`: both are settled at once, a monitor reads `r` after
-- `ext` rises and after `rv` changes while the reset holds, between edges.
-- Monitors outside print the registers 1 ns after each edge.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_lazy is
end entity;
architecture a of cycle_lazy is
  type table_t is array (0 to 3) of std_ulogic_vector(3 downto 0);
  constant TABLE : table_t := (x"1", x"2", x"4", x"8");
  signal clk : std_ulogic := '0';
  signal a, ae, b, x, q, q2 : std_ulogic := '0';
  signal valid, busy, ready : std_ulogic := '0';
  signal sel : natural range 0 to 3 := 0;
  signal t, tq : std_ulogic_vector(3 downto 0) := (others => '0');
  signal cnt : unsigned(3 downto 0) := (others => '0');
  signal handshakes : natural := 0;
  signal ext, arst, r, rv : std_ulogic := '0';
begin
  -- The region.
  comb : process (a, ae, b)
  begin
    x <= a xor ae xor b;
  end process;

  hs : process (valid, busy)
  begin
    ready <= valid and not busy;
  end process;

  look : process (sel)
  begin
    t <= TABLE(sel);
  end process;

  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= x;
      q2 <= q;
      tq <= t;
      cnt <= cnt + 1;
      busy <= cnt(1);
    end if;
  end process;

  mkrst : process (ext, q2)
  begin
    arst <= ext and not q2;
  end process;

  areg : process (clk, arst, rv)
  begin
    if arst = '1' then
      r <= rv;
    elsif rising_edge(clk) then
      r <= '1';
    end if;
  end process;

  -- Outside: pulses `ext` between edges and reads `r` 2 ns later.
  rmon : process
  begin
    for i in 1 to 8 loop
      wait until rising_edge(clk);
      wait for 4 ns;
      ext <= '1';
      wait for 1 ns;
      report "r=" & to_string(r);
      rv <= not rv;
      wait for 1 ns;
      report "r=" & to_string(r);
      ext <= '0';
    end loop;
    wait;
  end process;

  -- Outside: the clock, `ae` in the delta of the edge at every fourth edge, and `b`.
  main : process
  begin
    for i in 1 to 24 loop
      clk <= '1';
      if i mod 4 = 0 then
        ae <= not ae;
      end if;
      wait for 5 ns;
      clk <= '0';
      if i mod 3 = 0 then
        b <= not b;
      end if;
      wait for 5 ns;
    end loop;
    report "q=" & to_string(q) & " q2=" & to_string(q2) & " tq=" & to_hstring(tq)
      & " handshakes=" & to_string(handshakes);
    wait;
  end process;

  -- Outside: `a` after an edge and twice between two edges, `sel` between edges.
  stim : process
  begin
    for i in 1 to 6 loop
      wait until rising_edge(clk);
      a <= not a;
      wait for 2 ns;
      a <= not a;
      wait for 1 ns;
      a <= not a;
      sel <= (sel + 1) mod 4;
    end loop;
    wait;
  end process;

  -- Outside: a bus model that raises `valid` and waits for `ready` in the same time step.
  bus_model : process
  begin
    for i in 1 to 10 loop
      wait until rising_edge(clk);
      wait for 1 ns;
      valid <= '1';
      wait for 0 ns;
      wait for 0 ns;
      if ready = '1' then
        handshakes <= handshakes + 1;
      end if;
      valid <= '0';
    end loop;
    wait;
  end process;

  monitor : process
  begin
    wait until rising_edge(clk);
    wait for 1 ns;
    report "q=" & to_string(q) & " q2=" & to_string(q2) & " tq=" & to_hstring(tq) & " ready="
      & to_string(ready);
  end process;
end architecture;
