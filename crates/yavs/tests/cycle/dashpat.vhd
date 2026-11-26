-- #1200: a pattern holding '-' given to a function as a literal, a constant, a variable, a
-- record-constant field and a signal, and indexed in a loop. Under `--two-state` a named or passed
-- metavalue literal holds its `--init-x` bits; only a literal operand of `=` keeps its meaning.
-- The test fills in PAT, WM1, MOD and WIDTH.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_dash_dut is
  port (clk : in std_ulogic; a, ps : in std_ulogic_vector(WM1 downto 0); k0, k1, k2, k3, k4, k5, k6, k7, k8, k9, k10, k11 : out natural);
end entity;
architecture a of cycle_dash_dut is
  type rec_t is record
    pat : std_ulogic_vector(WM1 downto 0);
    n : natural;
  end record;
  type tab_t is array (0 to 1) of rec_t;
  constant tab : tab_t := (("PAT", 1), ((others => '1'), 2));
  constant pc : std_ulogic_vector(WM1 downto 0) := "PAT";
  function zeros(p : std_ulogic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in p'range loop
      if p(i) = '0' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;
  -- A '-' of the pattern matches anything, as the pattern's own elements tell (#1200).
  function match_f(x, p : std_ulogic_vector) return boolean is
    variable m : boolean := true;
  begin
    for i in x'range loop
      if (p(i) = '1') or (p(i) = '0') then
        m := m and (p(i) = x(i));
      end if;
    end loop;
    return m;
  end function;
begin
  process (clk)
    variable c : integer_vector(0 to 11) := (others => 0);
    variable pv : std_ulogic_vector(WM1 downto 0);
  begin
    if rising_edge(clk) then
      pv := "PAT";
      c(0) := c(0) + zeros("PAT");
      c(1) := c(1) + zeros(pc);
      c(2) := c(2) + zeros(pv);
      c(3) := c(3) + zeros(tab(0).pat);
      c(4) := c(4) + zeros(ps);
      if match_f(a, "PAT") then c(5) := c(5) + 1; end if;
      if match_f(a, pc) then c(6) := c(6) + 1; end if;
      if match_f(a, pv) then c(7) := c(7) + 1; end if;
      if match_f(a, tab(0).pat) then c(8) := c(8) + 1; end if;
      if match_f(a, ps) then c(9) := c(9) + 1; end if;
      for i in pc'range loop
        if pc(i) = '0' then c(10) := c(10) + 1; end if;
      end loop;
      if std_match(a, pc) then c(11) := c(11) + 1; end if;
      if std_match(a, "PAT") then c(11) := c(11) + 10; end if;
      if a = pc then c(11) := c(11) + 100; end if;
      if a /= "PAT" then c(11) := c(11) + 1000; end if;
      k0 <= c(0); k1 <= c(1); k2 <= c(2); k3 <= c(3); k4 <= c(4); k5 <= c(5);
      k6 <= c(6); k7 <= c(7); k8 <= c(8); k9 <= c(9); k10 <= c(10); k11 <= c(11);
    end if;
  end process;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_dash is
end entity;
architecture a of cycle_dash is
  signal clk : std_ulogic := '0';
  signal a : std_ulogic_vector(WM1 downto 0) := (others => '0');
  signal ps : std_ulogic_vector(WM1 downto 0) := "PAT";
  type nat_v is array (0 to 11) of natural;
  signal k : nat_v;
begin
  dut : entity work.cycle_dash_dut
    port map (clk, a, ps, k(0), k(1), k(2), k(3), k(4), k(5), k(6), k(7), k(8), k(9), k(10), k(11));
  main : process
  begin
    for i in 0 to 19 loop
      a <= not std_ulogic_vector(to_unsigned(i mod MOD, WIDTH));
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    for i in k'range loop
      report "k" & integer'image(i) & "=" & integer'image(k(i));
    end loop;
    wait;
  end process;
end architecture;
