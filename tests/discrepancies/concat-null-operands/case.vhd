-- LRM 2008 9.2.5: bounds of the concatenation of two null arrays: the right operand's.
entity top is end entity;
architecture a of top is
  function lft (v : bit_vector) return integer is begin return v'left; end function;
  function rgt (v : bit_vector) return integer is begin return v'right; end function;
begin
  process
    variable x : bit_vector(7 downto 0) := (others => '0');
  begin
    report "RES " & integer'image(lft(x(3 downto 4) & x(3 downto 4))) & " "
      & integer'image(rgt(x(3 downto 4) & x(3 downto 4)));
    wait;
  end process;
end architecture;
