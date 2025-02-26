pub fn find_free_id(task_ids: &[u32]) -> u32 {
    for i in 1.. {
        if !task_ids.contains(&i) {
            return i
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    use darkfi::Result;
    #[test]
    fn find_free_id_test() -> Result<()> {
        let mut ids: Vec<u32> = vec![1, 3, 8, 9, 10, 3];
        let ids_empty: Vec<u32> = vec![];
        let ids_duplicate: Vec<u32> = vec![1; 100];

        let find_id = find_free_id(&ids);

        assert_eq!(find_id, 2);

        ids.push(find_id);

        assert_eq!(find_free_id(&ids), 4);

        assert_eq!(find_free_id(&ids_empty), 1);

        assert_eq!(find_free_id(&ids_duplicate), 2);

        Ok(())
    }
}
