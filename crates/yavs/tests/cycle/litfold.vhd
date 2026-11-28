-- A87: under --two-state a constant read whole maps once per run, a (others => meta) of a length
-- known at run time maps without a per-element call: the bits stay those of the literal at each
-- read (two sites of one constant differ under random), a '-' kept by std_match, a 'Z' driven
-- onto a resolved signal kept, a table read by element in a loop, a vector and records.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_litfold_dut is
  port (clk : in std_ulogic; n : in natural; v : out std_ulogic_vector(63 downto 0);
        z : out std_logic);
end entity;

architecture rtl of cycle_litfold_dut is
  type rec_t is record
    a : std_ulogic_vector(3 downto 0);
    s : std_ulogic;
    k : natural;
  end record;
  type tab_t is array (0 to 2) of rec_t;
  constant init : rec_t := ("X1UW", 'X', 3);
  constant c : std_ulogic_vector(7 downto 0) := "X1U0-ZWL";
  constant tab : tab_t := (("1--0", 'U', 1), ("0X-1", '-', 2), ("UUUU", 'H', 0));
  constant zs : std_ulogic_vector(0 to 2) := "Z1Z";

  function pick(m : std_ulogic_vector) return std_ulogic_vector is
  begin
    return m;
  end function;
begin
  process (clk)
    variable r1, r2 : rec_t;
    variable tv : tab_t;
    variable w : std_ulogic_vector(15 downto 0);
    variable p : std_ulogic_vector(11 downto 0);
    variable m : std_ulogic_vector(2 downto 0);
    variable e : std_ulogic;
  begin
    if rising_edge(clk) then
      r1 := init;
      r2 := init;
      tv := tab;
      for i in 0 to 2 loop
        p(4 * i + 3 downto 4 * i) := pick(tab(i).a);
        if std_match("1100", tab(i).a) then
          m(i) := '1';
        else
          m(i) := '0';
        end if;
      end loop;
      w := (others => '1');
      w(n downto 0) := (others => 'X');
      w(n + 4 downto n + 1) := (others => '0');
      w(n + 6 downto n + 5) := (others => 'L');
      w(n + 8 downto n + 7) := (others => '-');
      e := '0';
      if c = "X1U0-ZWL" then
        e := '1';
      end if;
      v <= r1.a & r1.s & r2.a & r2.s & c & tv(1).a & tv(2).s & p & m & w
           & e & "000000000";
      z <= zs(n mod 3);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_litfold is
end entity;

architecture tb of cycle_litfold is
  signal clk : std_ulogic := '0';
  signal v : std_ulogic_vector(63 downto 0);
  signal z : std_logic;
  signal n : natural := 2;
begin
  dut : entity work.cycle_litfold_dut port map (clk, n, v, z);
  process
  begin
    clk <= '1';
    wait for 5 ns;
    report "v=" & to_string(v) & " z=" & std_logic'image(z);
    clk <= '0';
    n <= 4;
    wait for 5 ns;
    clk <= '1';
    wait for 5 ns;
    report "v=" & to_string(v) & " z=" & std_logic'image(z);
    wait;
  end process;
end architecture;
