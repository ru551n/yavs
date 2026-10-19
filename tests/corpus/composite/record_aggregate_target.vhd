-- difftest: top=record_aggregate_target
-- A record value assigned to an aggregate target (LRM 10.6.2.1): positional elements,
-- scalar and array fields, a function result, a qualified aggregate and a variable.
entity record_aggregate_target is
end entity;

architecture a of record_aggregate_target is
  type rec is record
    a : integer;
    b : boolean;
    c : bit_vector(3 downto 0);
  end record;
  function f return rec is
  begin
    return (7, true, "1010");
  end function;
begin
  process
    variable x : integer;
    variable y : boolean;
    variable z : bit_vector(3 downto 0);
    variable r : rec := (1, false, "0000");
  begin
    (x, y, z) := f;
    assert x = 7 and y and z = "1010" severity failure;
    (x, y, z) := rec'(1, false, "0110");
    assert x = 1 and not y and z = "0110" severity failure;
    (x, y, z) := rec'(c => "0011", b => true, a => 9);
    report "z = " & to_string(z) & " x = " & integer'image(x) & " y = " & boolean'image(y);
    (x, y, z) := r;
    assert x = 1 and not y and z = "0000" severity failure;
    wait;
  end process;
end architecture;
