-- difftest: top=aggregate_target_logic_fields std=2008 ref=hand
-- Aggregate targets of records with std_logic and std_logic_vector fields (#350, LRM 10.5.2.1,
-- 10.6.2.1), named and nested, variables and signals.
-- ref=hand: the first reference simulator assigns by position and ignores the field names.
library ieee; use ieee.std_logic_1164.all;
entity aggregate_target_logic_fields is end entity;
architecture a of aggregate_target_logic_fields is
  type rl is record v : std_logic_vector(3 downto 0); b : std_logic; end record;
  type rl2 is record r : rl; c : std_logic_vector(1 downto 0); end record;
  signal sq : std_logic_vector(3 downto 0); signal sc : std_logic; signal sd : std_logic_vector(1 downto 0);
begin
  process
    variable q : std_logic_vector(3 downto 0); variable c : std_logic; variable d : std_logic_vector(1 downto 0);
  begin
    (q, c) := rl'("1010", '1');
    assert q = "1010" and c = '1' severity failure;
    (b => c, v => q) := rl'("0110", '0');
    assert q = "0110" and c = '0' severity failure;
    ((q, c), d) := rl2'(("1111", 'Z'), "01");
    assert q = "1111" and c = 'Z' and d = "01" severity failure;
    (q, c) := rl'("1000", '1');
    (b => sc, v => sq) <= rl'("0110", '0');
    wait for 1 ns;
    assert sq = "0110" and sc = '0' severity failure;
    ((sq, sc), sd) <= rl2'(("1111", 'Z'), "01");
    wait for 1 ns;
    assert sq = "1111" and sc = 'Z' and sd = "01" severity failure;
    report "ok";
    wait;
  end process;
end architecture;
