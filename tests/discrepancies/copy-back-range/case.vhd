-- LRM 4.2.2.2: the value of an out or inout formal copied back to its actual belongs to the
-- subtype of the actual; 10 does not belong to `integer range 0 to 9` (#1175).
entity top is end entity;
architecture a of top is
  procedure set10 (variable o : out integer) is begin o := 10; end procedure;
begin
  process
    variable q : integer range 0 to 9 := 0;
  begin
    set10(q);
    report "RES q=" & integer'image(q);
    wait;
  end process;
end architecture;
