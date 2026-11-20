-- difftest: top=copy_back_range_entry std=2008 exit=1
-- The value of the actual of an inout formal belongs to the formal's subtype when it is
-- copied in (LRM 4.2.2.2, #1175): 20 is not a value of `integer range 0 to 9`.
entity copy_back_range_entry is end entity;
architecture a of copy_back_range_entry is
  procedure inc (variable x : inout integer range 0 to 9) is begin x := x + 1; end procedure;
begin
  process
    variable q : integer := 5;
  begin
    inc(q);
    report "q=" & integer'image(q);
    q := 20;
    inc(q);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
