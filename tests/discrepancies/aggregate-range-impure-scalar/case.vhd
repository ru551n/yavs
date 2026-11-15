-- A range choice and `others` of an integer array aggregate whose expressions call an impure function.
entity top is end entity;
architecture a of top is
  type iv is array (0 to 3) of integer;
begin
  process
    variable n : natural := 0;
    impure function t return natural is begin n := n + 1; return n; end function;
    variable v : iv;
  begin
    v := (0 to 1 => t, others => t);
    report integer'image(v(0)) & " " & integer'image(v(1)) & " " & integer'image(v(2)) & " " & integer'image(v(3));
    wait;
  end process;
end architecture;
