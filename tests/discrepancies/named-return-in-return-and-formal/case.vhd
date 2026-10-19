-- LRM 2019 4.2.1: the result subtype of a named-return function call in a return statement and
-- as the actual of a constrained formal.
entity top is end entity;
architecture a of top is
  function iota return r of integer_vector is
    variable v : r;
  begin
    for i in v'range loop v(i) := i; end loop;
    return v;
  end function;
  function twice return r of integer_vector is
    variable v : r;
  begin
    return iota;
  end function;
  procedure show(x : integer_vector(1 to 3)) is
  begin
    report "RES show " & integer'image(x(3));
  end procedure;
begin
  process
    variable v : integer_vector(1 to 3);
  begin
    show(iota);
    v := twice;
    report "RES twice " & integer'image(v(3));
    wait;
  end process;
end architecture;
