-- LRM 14.4.1, 8.7: a package is elaborated before the design hierarchy, so its declaration
-- cannot name an object of the hierarchy.
package p is
  constant c : integer := <<constant .top.k : integer>>;
end package;
use work.p.all;
entity top is end;
architecture t of top is
  constant k : integer := 5;
begin
  process
  begin
    report "RES " & integer'image(c);
    wait;
  end process;
end;
