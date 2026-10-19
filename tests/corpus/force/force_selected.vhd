-- difftest: top=force_selected std=2008 ref=hand
-- Selected force assignments (LRM 10.5.4) of a scalar, an array and a record (#68). ref=hand:
-- the reference simulator does not parse the selected force form; the expected output is the
-- LRM result, the same on the third and fourth reference simulators.
library ieee; use ieee.std_logic_1164.all;
entity force_selected is end entity;
architecture a of force_selected is
  type rec is record b : std_logic_vector(3 downto 0); n : integer; end record;
  signal x : std_logic_vector(3 downto 0) := "0000";
  signal r : rec := ("0000", 0);
  signal i : integer := 0;
begin
  process
    variable sel : integer := 2;
  begin
    with sel select x <= force "0001" when 1, "0010" when 2, "1111" when others;
    with sel select r <= force ("0001", 1) when 1, ("0010", 2) when 2, ("1111", 3) when others;
    with sel select i <= force 10 when 1, 20 when 2, 30 when others;
    wait for 1 ns;
    report "x=" & to_string(x) & " r.n=" & integer'image(r.n) & " i=" & integer'image(i);
    assert x = "0010" and r = ("0010", 2) and i = 20 severity failure;
    sel := 7;
    with sel select r.b <= force "0100" when 1 to 3, "1000" when others;
    wait for 1 ns;
    report "r.b=" & to_string(r.b);
    assert r.b = "1000" severity failure;
    x <= release; r <= release; i <= release;
    wait for 1 ns;
    report "released x=" & to_string(x) & " r.n=" & integer'image(r.n) & " i=" & integer'image(i);
    assert x = "0000" and r = ("0000", 0) and i = 0 severity failure;
    wait;
  end process;
end architecture;
