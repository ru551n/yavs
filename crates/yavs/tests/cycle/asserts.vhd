-- Asserts and reports below the top-level unit, evaluated in cycle mode (A52): a clocked process
-- at its edge (note, warning, error, a `report`, and the failure that ends the test), combinational
-- processes once their inputs are settled and only when what they wait on changed (a part of a
-- wide vector across a word boundary, one field of a record, an integer), a concurrent assert.
-- The testbench changes every input of one process in one delta, so the normal mode has no
-- intermediate values to report. `early` and `peer` report at the edge of the failure, before and
-- after `reg` in elaboration order. `wide` names more than it reads and stays in the event kernel,
-- and so does `rst_chk`, whose assert is in its reset branch.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_asserts_dut is
  port (
    clk  : in std_ulogic;
    rst  : in std_ulogic;
    a, b : in std_ulogic_vector(7 downto 0);
    w    : in std_ulogic_vector(W - 1 downto 0);
    i    : in integer range 0 to 1000;
    q    : out unsigned(7 downto 0)
  );
end entity;

architecture rtl of cycle_asserts_dut is
  type pair is record
    f1 : std_ulogic_vector(3 downto 0);
    f2 : std_ulogic_vector(3 downto 0);
  end record;
  type phase is (idle, busy, finished);
  signal n : unsigned(7 downto 0) := x"00";
  signal st : phase := idle;
  signal r : pair := (others => (others => '0'));
  signal m : std_ulogic := '0';

  -- Its assert runs also under `--no-asserts`, which drops only the processes' own (A54).
  procedure chk(x : unsigned) is
  begin
    assert x /= 11 report "chk sees 11" severity note;
  end procedure;
begin
  early : process (clk)
  begin
    if rising_edge(clk) then
      if n = 14 then
        report "early sees 14";
      end if;
    end if;
  end process;

  reg : process (clk)
  begin
    if rising_edge(clk) then
      n <= n + 1;
      chk(n);
      if n = 4 then
        st <= busy;
      elsif n = 8 then
        st <= finished;
      end if;
      assert n /= 5 report "n is " & integer'image(to_integer(n)) severity note;
      if n = 3 then
        report "three, a=" & to_hstring(a);
      end if;
      assert n /= 7 report "seven" severity warning;
      assert n /= 9 report "nine, an error" severity error;
      assert n /= 14 report "the design ends the test at " & to_string(n) severity failure;
    end if;
  end process;

  peer : process (clk)
  begin
    if rising_edge(clk) then
      if n = 14 then
        report "peer sees 14";
      end if;
    end if;
  end process;

  sum : process (all)
  begin
    report "a and b: " & to_hstring(a) & " " & to_hstring(b);
  end process;

  r <= (f1 => a(3 downto 0), f2 => b(3 downto 0));

  field : process (all)
  begin
    report "f1 is " & to_string(r.f1);
  end process;

  part : process (all)
  begin
    report "part " & to_string(w(W - 1 downto 1));
  end process;

  -- An enumeration made by `reg`: reported after the settle of the edge.
  stp : process (all)
  begin
    report "st is " & phase'image(st);
  end process;

  int : process (i)
  begin
    assert i < 300 report "i above 299: " & integer'image(i) severity warning;
  end process;

  assert unsigned(a) /= 3 report "a is 3 (concurrent)" severity warning;

  wide : process (a, b)
  begin
    m <= a(0);
    report "wide sees a(0)=" & std_ulogic'image(a(0));
  end process;

  rst_chk : process (clk, rst)
  begin
    if rst = '1' then
      report "in reset";
    elsif rising_edge(clk) then
      null;
    end if;
  end process;

  q <= n;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_asserts is
end entity;

architecture tb of cycle_asserts is
  signal clk : std_ulogic := '0';
  signal rst : std_ulogic := '1';
  signal a, b : std_ulogic_vector(7 downto 0) := x"00";
  signal w : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal i : integer range 0 to 1000 := 0;
  signal q : unsigned(7 downto 0);
begin
  dut : entity work.cycle_asserts_dut
    port map (clk => clk, rst => rst, a => a, b => b, w => w, i => i, q => q);

  clocks : process
  begin
    for k in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;

  main : process
  begin
    wait for 2 ns;
    rst <= '0';
    a <= x"01";
    b <= x"10";
    wait for 10 ns;
    a <= x"03";
    wait for 10 ns;
    -- Only f2 of the record and bit 0 of `w`, neither read by its process: no report.
    b <= x"11";
    w(0) <= '1';
    wait for 10 ns;
    w(W - 1) <= '1';
    i <= 400;
    wait for 10 ns;
    -- The same value again: no event.
    i <= 400;
    a <= x"03";
    wait for 10 ns;
    i <= 7;
    w <= (others => '1');
    wait;
  end process;
end architecture;
