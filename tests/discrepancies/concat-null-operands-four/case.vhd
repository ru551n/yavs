-- LRM 2008 9.2.5: bounds of the concatenation of null arrays: the right operand's.
-- Chained: ((e1 & e2) & e3) & e4 takes the bounds of e4, the last operand.
entity top is end entity;
architecture a of top is
  function lft (v : bit_vector) return integer is begin return v'left; end function;
  function rgt (v : bit_vector) return integer is begin return v'right; end function;
begin
  process
    variable x : bit_vector(7 downto 0) := (others => '0');
  begin
    report "RES " & integer'image(lft(x(6 downto 7) & x(5 downto 6) & x(3 downto 4) & x(1 downto 2))) & " "
      & integer'image(rgt(x(6 downto 7) & x(5 downto 6) & x(3 downto 4) & x(1 downto 2)));
    wait;
  end process;
end architecture;
