-- LRM 4.5.3, 6.5.7.1: two procedures distinguished only by the type of a formal conversion.
entity top is end entity;
architecture a of top is
  function cv (i : integer) return real is begin return real(i); end function;
  function cv (i : integer) return bit is begin return '1'; end function;
  procedure set (variable x : out integer; variable y : out integer) is begin x := 1; y := 2; end procedure;
begin
  process
    variable r : real;
    variable b : bit;
    variable q : integer;
  begin
    set(cv(x) => r, y => q);
    wait;
  end process;
end architecture;
