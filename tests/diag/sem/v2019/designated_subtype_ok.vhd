-- yavs-test: std=2019
-- expect: none
-- LRM 16.2.2 (2019): T'DESIGNATED_SUBTYPE of an access and of a file type
package p is
  subtype byte is integer range 0 to 255;
  type bp is access byte;
  type bf is file of character;
  subtype b2 is bp'designated_subtype;
  subtype c2 is bf'designated_subtype;
  constant x : b2 := 255;
  constant y : c2 := 'a';
end package;
