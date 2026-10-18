entity e is
  generic (type t is private);
  port (a : in bit;);
end entity;

package p is
  component c is
    port (a : bit);
  end;
end package;
