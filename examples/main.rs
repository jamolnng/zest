extern crate zest;

use zest::pkzip::ZipArchive;

fn main() {
  match ZipArchive::open("./test.zip") {
    Ok(archive) => {
      println!("{:#X?}", archive);
    }
    Err(e) => println!("{:#?}", e),
  }
}
