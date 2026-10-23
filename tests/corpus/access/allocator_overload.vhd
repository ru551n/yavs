-- difftest: top=allocator_overload ref=hand
-- ref=hand: the first reference simulator cannot determine the type of the allocators, the
-- second runs it. An allocator has an access type designating its subtype's type, so the "="
-- of two allocators of INTEGER is the one of the access type of INTEGER (LRM 9.3.7, 12.5, #27).
entity allocator_overload is end;
architecture a of allocator_overload is
  type ia is access integer;
  type ba is access bit;
begin
  process
  begin
    if new integer = new integer then
      report "same";
    else
      report "different";
    end if;
    if new bit'('1') /= null then
      report "not null";
    end if;
    wait;
  end process;
end;
