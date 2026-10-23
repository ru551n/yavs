-- LRM 9.3.6, 5.2.1: a universal integer literal converted to INTEGER lies in its range;
-- outside is an error when evaluated, here as the argument of 'image (2008: 32 bits).
entity top is end entity;
architecture a of top is
begin
  process
  begin
    report "RES " & integer'image(2147483647);
    report "RES " & integer'image(-2147483648);
    report "RES " & integer'image(2147483648);
    report "RES not reached";
    wait;
  end process;
end architecture;
