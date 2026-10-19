-- A package with a body declared in an architecture (LRM 4.7, 4.8, 2008): its constant
-- and function are used by a process.
entity top is end;
architecture a of top is
  package p is
    constant c : integer := 9;
    function plus(x : integer) return integer;
  end package;
  package body p is
    function plus(x : integer) return integer is
    begin
      return x + c;
    end function;
  end package body;
begin
  process
  begin
    report "RES " & integer'image(p.plus(1));
    wait;
  end process;
end;
