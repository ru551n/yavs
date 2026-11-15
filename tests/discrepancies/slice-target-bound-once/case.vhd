-- A slice target whose bound calls an impure function, with an aggregate value that takes its
-- bounds from the target (LRM 10.5.2.1, 10.6.2.1: the name is evaluated once): the number of
-- calls per assignment, for variable and signal targets.
entity top is end entity;
architecture t of top is
  type iv is array (natural range <>) of integer;
  signal s : iv(0 to 3);
begin
  process is
    variable v, w : iv(0 to 3);
    variable n : natural := 0;
    impure function t(k : natural) return natural is
    begin
      n := n + 1;
      return k;
    end function;
  begin
    n := 0; v(t(1) to 2) := w(0 to 1);
    report "RES var slice, plain value: " & integer'image(n);
    n := 0; v(t(1) to 2) := (5, 6);
    report "RES var slice, aggregate: " & integer'image(n);
    n := 0; s(t(1) to 2) <= (5, 6);
    report "RES sig slice, aggregate: " & integer'image(n);
    n := 0; s(t(0) to 3) <= (others => 7), (others => 8) after 1 ns;
    report "RES sig slice, two elements: " & integer'image(n);
    wait;
  end process;
end architecture;
