-- difftest: top=generic_entity_locals_2019 std=2019
-- A generic type in the architecture's own array type, function, signal and variable.
entity acc is
  generic (type t; z : t; function add(a, b : t) return t; function img(x : t) return string; n : positive);
end;
architecture a of acc is
  type arr is array (1 to n) of t;
  function sum(v : arr) return t is
    variable s : t := z;
  begin
    for i in v'range loop s := add(s, v(i)); end loop;
    return s;
  end;
  signal v : arr := (others => z);
begin
  process
    variable tmp : arr;
  begin
    for i in tmp'range loop tmp(i) := v(i); end loop;
    report img(sum(tmp)) & " " & integer'image(arr'length);
    wait;
  end process;
end;
entity generic_entity_locals_2019 is end;
architecture t of generic_entity_locals_2019 is
  function addi(a, b : integer) return integer is begin return a + b + 1; end;
  function ii(x : integer) return string is begin return integer'image(x); end;
  function addr(a, b : real) return real is begin return a + b + 0.5; end;
  function ri(x : real) return string is begin return integer'image(integer(x * 10.0)); end;
begin
  u1 : entity work.acc generic map (t => integer, z => 0, add => addi, img => ii, n => 3);
  u2 : entity work.acc generic map (t => real, z => 1.0, add => addr, img => ri, n => 4);
end;
