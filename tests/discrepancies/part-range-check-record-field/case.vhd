-- LRM 6.5.6.3, 14.7.3.1: the equivalent process of an expression actual of a record field in
-- an individual association checks the value against the field's subtype: 8 is outside
-- 0 to 7, so the run stops with a range error at 1 ns.
package pk is
  type rec is record f : integer range 0 to 7; g : bit; end record;
end package;
use work.pk.all;
entity mon is port (r : in rec); end entity;
architecture a of mon is
begin
  process (r) begin
    if now > 0 ns then report "RES r.f=" & integer'image(r.f); end if;
  end process;
end architecture;
use work.pk.all;
entity top is end entity;
architecture a of top is
  signal x : integer := 1;
begin
  m : entity work.mon port map (r.f => x + 1, r.g => '0');
  process begin wait for 1 ns; x <= 7; wait; end process;
end architecture;
