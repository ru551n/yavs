-- LRM 6.5.7.1: an inout port with a conversion on one side only (id(q) => io).
entity sub is port (q : inout integer); end entity;
architecture a of sub is begin q <= 3; end architecture;
entity top is end entity;
architecture a of top is
  signal io : integer := 0;
  function id(v : integer) return integer is begin return v + 1; end function;
begin
  u : entity work.sub port map (id(q) => io);
  process begin wait for 1 ns; report "RES " & integer'image(io); wait; end process;
end architecture;
