-- A generic package with a body declared in an architecture and instantiated after its
-- body in the same architecture (LRM 4.9 has no rule against it).
entity top is end;
architecture a of top is
  package g is
    generic (k : integer);
    function scale(x : integer) return integer;
  end package;
  package body g is
    function scale(x : integer) return integer is
    begin
      return k * x;
    end function;
  end package body;
  package by3 is new g generic map (k => 3);
begin
  process
  begin
    report "RES " & integer'image(by3.scale(5));
    wait;
  end process;
end;
