-- difftest: top=static_intermediate
-- A static integer expression keeps its exact value up to the outermost operation of its type:
-- 2 ** 31 - 1 is INTEGER'HIGH although 2 ** 31 is outside INTEGER (#872). The range check is
-- at the value the arithmetic produces: a port, signal and variable range, a condition, a
-- constant, a negative bound, a generic in a process, a named constant operand.
entity static_intermediate is
  generic (g : natural := 31);
  port (cnt : out natural range 0 to 2 ** 31 - 1 := 0);
end entity;

architecture a of static_intermediate is
  type small is range 0 to 100;
  constant c_small : small := 200 - 150;
  constant c_small2 : small := (60 + 60) - 50;
  constant w : integer := g + 1;
  signal s : natural range 0 to 2 ** 31 - 1 := 2 ** 31 - 1;
  signal sg : natural range 0 to 2 ** g - 1;
  constant c_low : integer := -2 ** 31;
  constant c_high : integer := 2 ** 31 - 1;
  constant k : integer := 2;
  constant c_named : integer := k ** 31 - 1;
  constant c_div : integer := 2 ** 40 / 2 ** 20;
  constant c_time : time := (2 ** 31 - 1) * 1 fs;
begin
  process
    variable v : natural range 0 to 2 ** 31 - 1 := 5;
  begin
    assert c_high = integer'high severity failure;
    assert c_low = integer'low severity failure;
    assert c_named = integer'high severity failure;
    assert c_div = 2 ** 20 severity failure;
    if v = 0 then
      v := 1;
    elsif v < 2 ** 31 - 1 then
      report "condition";
    end if;
    v := 2 ** 31 - 1;
    assert v = s severity failure;
    report integer'image(2 ** g - 1) & " " & integer'image(-(2 ** g)) & " " & integer'image(c_low);
    report integer'image(abs (-(2 ** 31) + 1)) & " " & integer'image(2 ** 31 / 2 - 2 ** 31 / 4);
    report small'image(c_small) & " " & small'image(c_small2) & " " & time'image(c_time);
    report integer'image(2 ** (w - 1) - 1);
    wait;
  end process;
end architecture;
