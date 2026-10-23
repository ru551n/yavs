-- yavs-test: std=2008
-- expect: none
-- LRM 9.3.7, 12.5 (the type of an allocator is an access type designating its subtype's type: of the access types' predefined "=", only one fits two allocators of INTEGER; sim2 accepts, sim1 cannot determine the type)
entity e is end entity;
architecture a of e is
  type ia is access integer;
  type ba is access bit;
begin
  process
  begin
    if new integer = new integer then
      report "same";
    end if;
    if new bit'('1') /= null then
      report "not null";
    end if;
    wait;
  end process;
end architecture;
