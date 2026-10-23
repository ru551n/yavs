-- LRM 9.3.7, 12.5: an allocator has an access type designating the type of its subtype
-- indication, so the "=" of two allocators of INTEGER is the one of the access type of INTEGER.
entity top is end entity;
architecture a of top is
  type ia is access integer;
  type ba is access bit;
begin
  process
  begin
    if new integer = new integer then
      report "RES same";
    else
      report "RES different";
    end if;
    if new bit'('1') /= null then
      report "RES not null";
    end if;
    wait;
  end process;
end architecture;
