-- LRM 5.3.2.2: unconstrained ports of an array of records, converted with constrained type
-- marks: the in port takes 1 to 3 from `rb3`, the out port 7 downto 5 from `ra3`.
package pk is
  type rec is record a : bit; n : integer; end record;
  type ra is array (natural range <>) of rec;
  type rb is array (natural range <>) of rec;
  subtype rb3 is rb(1 to 3);
  subtype ra3 is ra(7 downto 5);
end package;
use work.pk.all;
entity sin is port (p : in rb); end entity;
architecture a of sin is
begin
  process begin
    report "RES p " & integer'image(p'left) & " " & integer'image(p'right) & " "
      & integer'image(p(p'left).n) & integer'image(p(p'right).n);
    wait;
  end process;
end architecture;
use work.pk.all;
entity sout is port (q : out rb); end entity;
architecture a of sout is
begin
  process begin
    report "RES q " & integer'image(q'left) & " " & integer'image(q'right);
    for k in q'range loop q(k) <= ('1', 5); end loop;
    q(q'left) <= ('0', 1);
    wait;
  end process;
end architecture;
use work.pk.all;
entity top is end entity;
architecture a of top is
  signal sa : ra(2 downto 0) := (('1', 1), ('0', 2), ('1', 3));
  signal so : ra(2 downto 0);
begin
  i1 : entity work.sin port map (p => rb3(sa));
  o1 : entity work.sout port map (ra3(q) => so);
  process begin
    wait for 1 ns;
    report "RES so " & integer'image(so(2).n) & integer'image(so(0).n);
    wait;
  end process;
end architecture;
