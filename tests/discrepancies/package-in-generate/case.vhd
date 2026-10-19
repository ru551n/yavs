-- A package declared in a generate statement's declarative part (LRM 11.8: a block
-- declarative part, 3.3.2 lists package declarations), one per iteration.
entity top is end;
architecture a of top is
begin
  g : for i in 1 to 2 generate
    package per is
      constant v : integer := i * 100;
    end package;
  begin
    process
    begin
      report "RES " & integer'image(per.v);
      wait;
    end process;
  end generate;
end;
