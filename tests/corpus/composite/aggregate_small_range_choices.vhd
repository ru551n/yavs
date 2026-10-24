-- difftest: top=aggregate_small_range_choices std=2008
-- Aggregates whose choices are ranges of a few static elements (the carry-in idiom
-- `(0 to 0 => c)`, `(3 downto 2 => x)`), on logic vectors and on other element types, up and
-- down, null ranges, several choices, `others`, and a range longer than the few.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity aggregate_small_range_choices is
end entity;

architecture t of aggregate_small_range_choices is
  type int_vec is array (natural range <>) of integer;
  signal c : std_logic := '1';
begin
  process
    variable u1 : unsigned(0 downto 0);
    variable u2 : unsigned(0 to 0);
    variable v4 : std_logic_vector(3 downto 0);
    variable v5 : std_logic_vector(0 to 4);
    variable v7 : std_logic_vector(6 downto 0);
    variable iv : int_vec(2 to 5);
    variable bv : bit_vector(1 to 4);
    variable x  : integer := 7;
  begin
    u1 := (0 downto 0 => c);
    u2 := (0 to 0 => not c);
    v4 := (3 downto 2 => '1', 1 downto 0 => '0');
    v4 := (3 downto 3 => 'H', 2 downto 1 => 'Z', others => 'X');
    v5 := (0 to 1 => '1', 2 to 4 => '0');
    v7 := (6 downto 4 => '1', 3 downto 0 => c);
    iv := (2 to 3 => x, 4 to 5 => x + 1);
    bv := (1 to 2 => '1', 3 to 4 => '0');
    report "a " & to_string(u1) & " " & to_string(u2) & " " & to_string(v4) & " " & to_string(v5)
      & " " & to_string(v7);
    report "b " & integer'image(iv(2)) & integer'image(iv(3)) & integer'image(iv(4))
      & integer'image(iv(5)) & " " & to_string(bv);
    -- null ranges: nothing is written, the rest is `others`
    v4 := (3 downto 4 => '1', others => '0');
    v5 := (1 to 0 => '1', 0 to 4 => 'W');
    report "c " & to_string(v4) & " " & to_string(v5);
    -- a longer range (a loop) next to short ones
    v7 := (6 downto 2 => '1', 1 downto 0 => '0');
    report "d " & to_string(v7);
    -- the element value is evaluated once per aggregate
    iv := (2 to 3 => x * 2, others => 0);
    report "e " & integer'image(iv(2)) & integer'image(iv(3)) & integer'image(iv(4));
    wait;
  end process;
end architecture;
