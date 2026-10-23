-- A concurrent assignment that reads its own target.
entity stuck_comb is end entity;
architecture a of stuck_comb is
  signal a : bit := '0';
begin
  a <= not a;
end architecture;
