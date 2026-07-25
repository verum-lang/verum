//! Comprehensive examples of semantic types usage
//!
//! Run with: cargo run --example semantic_types_usage --package verum_common

use verum_common::semantic_types::{List, Map, OrderedMap, OrderedSet, Set, Text};

fn main() {
    println!("=== Semantic Types Examples ===\n");

    text_examples();
    list_examples();
    map_examples();
    set_examples();
    ordered_collections_examples();
    integration_examples();
}

fn text_examples() {
    println!("--- TEXT EXAMPLES ---");

    // Construction
    let mut text = Text::from("Hello");
    println!("Created text: {}", text);

    // Mutation
    text.push_str(" World");
    text.push('!');
    println!("After mutation: {}", text);

    // Splitting
    let sentence = Text::from("The quick brown fox");
    let words = sentence.split(" ");
    println!("Words: {:?}", words);

    // Transformation
    let lower = text.to_lowercase();
    let upper = text.to_uppercase();
    println!("Lowercase: {}, Uppercase: {}", lower, upper);

    // Pattern matching
    let email = Text::from("user@example.com");
    if email.contains("@") {
        let parts = email.split("@");
        println!("Email user: {}, domain: {}", parts[0], parts[1]);
    }

    // Trimming and padding
    let spaced = Text::from("  trim me  ");
    let trimmed = spaced.trim();
    let padded = Text::from("42").pad_left(5, '0');
    println!("Trimmed: '{}', Padded: '{}'", trimmed, padded);

    println!();
}

fn list_examples() {
    println!("--- LIST EXAMPLES ---");

    // Construction
    let numbers = List::from(vec![1, 2, 3, 4, 5]);
    println!("Initial list: {:?}", numbers);

    // Functional operations
    let doubled = numbers.clone().map(|x| x * 2);
    println!("Doubled: {:?}", doubled);

    let evens = numbers.clone().filter(|&x| x % 2 == 0);
    println!("Evens: {:?}", evens);

    // Sorting and transformation
    let mut unsorted = List::from(vec![5, 2, 8, 1, 9]);
    unsorted.sort();
    println!("Sorted: {:?}", unsorted);

    unsorted.reverse();
    println!("Reversed: {:?}", unsorted);

    // Windowing
    let data = List::from(vec![1, 2, 3, 4, 5]);
    let windows: Vec<_> = data.windows(3).collect();
    println!("Windows of 3: {:?}", windows);

    // Joining
    let items = List::from(vec![1, 2, 3]);
    let joined = items.join(", ");
    println!("Joined: {}", joined);

    println!();
}

fn map_examples() {
    println!("--- MAP EXAMPLES ---");

    // Create a phonebook
    let mut phonebook = Map::new();
    phonebook.insert(Text::from("Alice"), Text::from("555-1234"));
    phonebook.insert(Text::from("Bob"), Text::from("555-5678"));
    phonebook.insert(Text::from("Charlie"), Text::from("555-9012"));

    println!("Phonebook entries:");
    for (name, number) in phonebook.iter() {
        println!("  {}: {}", name, number);
    }

    // Lookup
    if let Some(number) = phonebook.get(&Text::from("Alice")) {
        println!("Alice's number: {}", number);
    }

    // Update using entry API
    phonebook
        .entry(Text::from("Alice"))
        .and_modify(|n| *n = Text::from("555-0000"));
    println!(
        "Updated Alice's number to: {}",
        phonebook[&Text::from("Alice")]
    );

    // Count word frequencies
    let text = Text::from("hello world hello rust hello");
    let words = text.split(" ");
    let mut freq = Map::new();
    for word in words {
        *freq.entry(word.clone()).or_insert(0) += 1;
    }
    println!("\nWord frequencies:");
    for (word, count) in freq.iter() {
        println!("  '{}': {}", word, count);
    }

    println!();
}

fn set_examples() {
    println!("--- SET EXAMPLES ---");

    // Create sets
    let set1: Set<i32> = [1, 2, 3, 4, 5].into_iter().collect();
    let set2: Set<i32> = [4, 5, 6, 7, 8].into_iter().collect();

    println!("Set 1: {:?}", set1);
    println!("Set 2: {:?}", set2);

    // Set operations
    let union: Vec<_> = set1.union(&set2).copied().collect();
    println!("Union: {:?}", union);

    let intersection: Vec<_> = set1.intersection(&set2).copied().collect();
    println!("Intersection: {:?}", intersection);

    let difference: Vec<_> = set1.difference(&set2).copied().collect();
    println!("Difference (1-2): {:?}", difference);

    // Membership tests
    println!("Set1 contains 3: {}", set1.contains(&3));
    println!("Set1 contains 10: {}", set1.contains(&10));

    // Deduplication using Set
    let duplicates = List::from(vec![1, 2, 2, 3, 3, 3, 4, 4, 4, 4]);
    let unique: Set<_> = duplicates.into_iter().collect();
    println!("Deduplicated: {:?}", unique);

    println!();
}

fn ordered_collections_examples() {
    println!("--- ORDERED COLLECTIONS EXAMPLES ---");

    // OrderedMap - maintains sorted keys
    let mut scores = OrderedMap::new();
    scores.insert(3, Text::from("Charlie"));
    scores.insert(1, Text::from("Alice"));
    scores.insert(2, Text::from("Bob"));

    println!("Leaderboard (sorted by score):");
    for (score, name) in scores.iter() {
        println!("  {}: {}", score, name);
    }

    // OrderedSet - maintains sorted elements
    let mut sorted_numbers = OrderedSet::new();
    sorted_numbers.insert(5);
    sorted_numbers.insert(1);
    sorted_numbers.insert(3);
    sorted_numbers.insert(2);
    sorted_numbers.insert(4);

    println!("\nSorted numbers:");
    for num in sorted_numbers.iter() {
        print!("{} ", num);
    }
    println!();

    // Range operations
    if let Some(first) = sorted_numbers.first() {
        println!("First element: {}", first);
    }
    if let Some(last) = sorted_numbers.last() {
        println!("Last element: {}", last);
    }

    println!();
}

fn integration_examples() {
    println!("--- INTEGRATION EXAMPLES ---");

    // Parse CSV data
    let csv = Text::from("name,age,city\nAlice,30,NYC\nBob,25,LA\nCharlie,35,SF");
    let lines = csv.lines();

    let mut data: List<Map<Text, Text>> = List::new();
    let headers = lines[0].split(",");

    for line in lines.iter().skip(1) {
        let values = line.split(",");
        let mut row = Map::new();
        for (i, value) in values.iter().enumerate() {
            if let Some(header) = headers.get(i) {
                row.insert(header.clone(), value.clone());
            }
        }
        data.push(row);
    }

    println!("Parsed CSV data:");
    for (i, row) in data.iter().enumerate() {
        println!("  Row {}:", i);
        for (key, value) in row.iter() {
            println!("    {}: {}", key, value);
        }
    }

    // Configuration parser
    let config = Text::from(
        "# Database config\n\
         host=localhost\n\
         port=5432\n\
         # User settings\n\
         username=admin",
    );

    let mut settings = Map::new();
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("#") {
            continue;
        }
        let parts = trimmed.split("=");
        if parts.len() == 2 {
            settings.insert(parts[0].trim(), parts[1].trim());
        }
    }

    println!("\nParsed configuration:");
    for (key, value) in settings.iter() {
        println!("  {} = {}", key, value);
    }

    // Data aggregation
    let sales = List::from(vec![
        (Text::from("Product A"), 100),
        (Text::from("Product B"), 200),
        (Text::from("Product A"), 150),
        (Text::from("Product C"), 75),
        (Text::from("Product B"), 125),
    ]);

    let mut totals = Map::new();
    for (product, amount) in sales.iter() {
        *totals.entry(product.clone()).or_insert(0) += amount;
    }

    println!("\nTotal sales by product:");
    for (product, total) in totals.iter() {
        println!("  {}: ${}", product, total);
    }

    println!();
}
