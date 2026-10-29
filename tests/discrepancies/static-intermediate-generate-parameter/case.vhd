-- LRM 9.3.6, 5.2.3.1: 2 ** j with j = 31 lies outside INTEGER. A generate parameter is not
-- taken as known at elaboration (unlike a generic, L156): each operation is checked (#872).
entity top is end entity;
architecture a of top is
begin
  gen: for j in 31 to 31 generate
    process
    begin
      report "RES " & integer'image(2 ** j - 1);
      wait;
    end process;
  end generate;
end architecture;
