-- difftest: top=drive_var_later std=2008
-- A signal assigned a variable's value keeps that value when the variable is assigned again
-- later in the same run and the signal may be assigned again after that (LRM 10.5.2.1: the
-- waveform is evaluated when the assignment statement executes). An optimisation that delays
-- the first drive to where the second one may replace it had moved the copy of the variable
-- with it, so the signal took the variable's new value.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity drive_var_later is
end entity;
architecture a of drive_var_later is
  type r_t is record
    f0 : std_ulogic;
    f2 : signed(31 downto 0);
  end record;
  signal clk, c : std_ulogic := '0';
  signal x : signed(31 downto 0) := x"12345678";
  signal s1, s2, s3 : signed(31 downto 0) := (others => '0');
  signal sv : std_ulogic_vector(7 downto 0) := (others => '0');
  signal q : r_t := ('0', (others => '0'));
begin
  clk <= not clk after 5 ns;

  p1 : process (clk)
    variable v : signed(31 downto 0) := (others => '0');
  begin
    if falling_edge(clk) then
      s1 <= v;
      v := x;
      if c = '1' then
        s1 <= (others => '0');
      end if;
    end if;
  end process;

  p2 : process
    variable v : signed(31 downto 0) := (others => '0');
  begin
    wait until rising_edge(clk);
    s2 <= v;
    v := x;
    if c = '1' then
      s2 <= (others => '1');
    end if;
  end process;

  p3 : process (clk)
    variable v : signed(31 downto 0) := (others => '0');
    variable b : std_ulogic_vector(7 downto 0) := (others => '0');
  begin
    if falling_edge(clk) then
      q.f2 <= v;
      sv <= b;
      s3(15 downto 0) <= v(15 downto 0);
      v := x;
      b := std_ulogic_vector(x(7 downto 0));
      case c is
        when '1' =>
          q.f2 <= (others => '0');
          sv <= (others => '0');
          s3(15 downto 0) <= (others => '0');
        when others =>
          null;
      end case;
    end if;
  end process;

  stim : process
  begin
    for i in 0 to 5 loop
      wait for 7 ns;
      report "s1=" & to_hstring(s1) & " s2=" & to_hstring(s2) & " s3=" & to_hstring(s3)
        & " q=" & to_hstring(q.f2) & " sv=" & to_hstring(sv);
      x <= x + 1;
      c <= '1' when i = 3 else '0';
      wait for 3 ns;
    end loop;
    std.env.finish;
  end process;
end architecture;
