-- LRM 4.6, 14.7.3.2: the bounds of the array of driving values a resolution function
-- receives. The LRM gives none; the index subtype here is descending. The resolved value
-- encodes left, right and direction.
entity top is end entity;
architecture a of top is
  subtype down is integer range 10 downto 1;
  type down_vec is array (down range <>) of integer;
  function bounds(x : down_vec) return integer is
  begin
    if x'ascending then
      return x'left * 1000 + x'right * 10 + 1;
    end if;
    return x'left * 1000 + x'right * 10;
  end function;
  signal s : bounds integer := 0;
begin
  s <= 0;
  s <= 0;
  process
  begin
    wait for 1 ns;
    report "RES left*1000 + right*10 + ascending = " & integer'image(s);
    wait;
  end process;
end architecture;
