-- LRM 12.3, 4.10: two instances of one generic procedure whose actuals are a type and a
-- subtype of it have the same profile, so they are homographs in one region (L89).
package p is
  procedure gen generic (type t) (x : t);
end package;

package body p is
  procedure gen generic (type t) (x : t) is
  begin
  end procedure;
end package body;

use work.p.all;
entity top is
end entity;

architecture a of top is
  subtype sub is bit;
  procedure inst is new gen generic map (t => bit);
  procedure inst is new gen generic map (t => sub);
begin
end architecture;
