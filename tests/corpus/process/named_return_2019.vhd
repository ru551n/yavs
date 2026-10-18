-- difftest: top=named_return_2019 std=2019
-- 2019 named return values (4.2.1, LCS-2016-072b): `return r of t` makes `r` the result
-- subtype, which a call takes from its context: variable and signal targets and initial
-- values. yavs also takes it from a constrained formal and from a `return` in another
-- named-return function; the reference simulator rejects those, so they are left out.
library ieee;
use ieee.std_logic_1164.all;
entity named_return_2019 is end;
architecture t of named_return_2019 is
  function iota return r of integer_vector is
    variable v : r;
    variable n : natural := 0;
  begin
    for i in v'range loop
      v(i) := n;
      n := n + 1;
    end loop;
    return v;
  end function;

  function fit(x : std_logic_vector) return rv of std_logic_vector is
    alias xa : std_logic_vector(x'length - 1 downto 0) is x;
    variable v : rv := (others => '0');
    alias va : std_logic_vector(v'length - 1 downto 0) is v;
  begin
    if v'length < x'length then
      va := xa(va'range);
    else
      va(xa'range) := xa;
    end if;
    return v;
  end function;

  signal s1 : integer_vector(1 to 4);
  signal s2 : integer_vector(6 to 7) := iota;
  signal s3 : std_logic_vector(5 downto 0);
begin
  process
    variable v1 : integer_vector(1 to 3);
    variable v2 : std_logic_vector(1 to 3);
    variable v3 : std_logic_vector(7 downto 0);
    constant c1 : std_logic_vector(0 to 1) := fit("1101");
  begin
    v1 := iota;
    report "v1 " & to_string(v1(1)) & to_string(v1(2)) & to_string(v1(3));
    report "s2 " & to_string(s2(6)) & to_string(s2(7));
    v2 := fit("1010101");
    report "v2 " & to_string(v2);
    v3 := fit("1");
    report "v3 " & to_string(v3);
    report "c1 " & to_string(c1);
    s1 <= iota;
    s3 <= fit("111000111");
    wait for 1 ns;
    report "s1 " & to_string(s1(1)) & to_string(s1(4));
    report "s3 " & to_string(s3);
    wait;
  end process;
end;
