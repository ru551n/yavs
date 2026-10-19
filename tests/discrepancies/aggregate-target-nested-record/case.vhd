-- LRM 10.6.2.1: an element association of an aggregate target that is itself an aggregate target (record of records).
entity top is end entity;
architecture a of top is
  type rec is record a, b : integer; end record;
  type rec2 is record r : rec; c : integer; end record;
  function g return rec2 is begin return ((3, 4), 5); end function;
begin
  process
    variable x, y, z : integer;
  begin
    ((x, y), z) := g;
    report "RES x=" & integer'image(x) & " y=" & integer'image(y) & " z=" & integer'image(z);
    wait;
  end process;
end architecture;
