-- yavs-test: std=2008
-- expect: E1301 11:8 no access type designates INTEGER and BIT
-- LRM 9.3.7, 12.5 (the operands of "=" are allocators of different designated types)
entity e is end entity;
architecture a of e is
  type ia is access integer;
  type ba is access bit;
begin
  process
  begin
    if new integer = new bit'('1') then
      report "same";
    end if;
    wait;
  end process;
end architecture;
