-- Targets of vhpi_put_value(vhpiStrVal) with fewer or more characters than the array has
-- elements (#345): string signals of both directions without drivers.
entity strput is end entity;

architecture sim of strput is
  signal up   : string(1 to 8) := "abcdefgh";
  signal down : string(8 downto 1) := "abcdefgh";
begin
end architecture;
