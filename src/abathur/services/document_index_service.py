"""Document index service for semantic search and file tracking."""

import hashlib
import json
from typing import Any

from abathur.infrastructure.database import Database


class DocumentIndexService:
    """Service for managing document indexing and semantic search.

    Tracks markdown files, content hashes, embeddings, and sync status
    for semantic search capabilities.
    """

    def __init__(self, db: Database) -> None:
        """Initialize document index service.

        Args:
            db: Database instance for storage operations
        """
        self.db = db

    async def index_document(
        self,
        file_path: str,
        title: str,
        content: str,
        document_type: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> int:
        """Index a new document with content hash.

        Args:
            file_path: Unique file path (acts as primary key)
            title: Document title
            content: Document content for hash generation
            document_type: Optional document type (design|specification|plan|report)
            metadata: Optional metadata dictionary

        Returns:
            Document ID

        Raises:
            ValueError: If file_path already exists

        Example:
            >>> doc_id = await document_service.index_document(
            ...     file_path="/docs/schema-design.md",
            ...     title="Schema Design",
            ...     content="# Schema Design\\n...",
            ...     document_type="design"
            ... )
        """
        # Generate content hash
        content_hash = hashlib.sha256(content.encode()).hexdigest()

        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    try:
                        cursor = await conn.execute(
                            """
                            INSERT INTO document_index (
                                file_path, title, document_type, content_hash, metadata, sync_status
                            )
                            VALUES (?, ?, ?, ?, ?, 'pending')
                            """,
                            (
                                file_path,
                                title,
                                document_type,
                                content_hash,
                                json.dumps(metadata or {}),
                            ),
                        )
                        doc_id = cursor.lastrowid
                        assert doc_id is not None, "Failed to get doc_id after insert"
                        await conn.commit()
                        return int(doc_id)
                    except Exception as e:
                        if "UNIQUE constraint failed" in str(e):
                            raise ValueError(f"Document already exists: {file_path}") from e
                        raise
            except Exception:
                await conn.rollback()
                raise

    async def update_embedding(
        self,
        doc_id: int,
        embedding: bytes,
        embedding_model: str = "nomic-embed-text-v1.5",
    ) -> None:
        """Update document embedding (BLOB storage for future vector search).

        Args:
            doc_id: Document ID
            embedding: Vector embedding as bytes
            embedding_model: Model used for embedding generation

        Example:
            >>> embedding_bytes = serialize_vector(embedding)
            >>> await document_service.update_embedding(doc_id, embedding_bytes)
        """
        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    await conn.execute(
                        """
                        UPDATE document_index
                        SET embedding_blob = ?, embedding_model = ?, updated_at = CURRENT_TIMESTAMP
                        WHERE id = ?
                        """,
                        (embedding, embedding_model, doc_id),
                    )
                    await conn.commit()
            except Exception:
                await conn.rollback()
                raise

    async def get_document(self, file_path: str) -> dict[str, Any] | None:
        """Retrieve document by file path.

        Args:
            file_path: Document file path

        Returns:
            Document dict with parsed metadata, or None if not found

        Example:
            >>> doc = await document_service.get_document("/docs/schema-design.md")
            >>> print(doc['sync_status'])  # 'pending'
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM document_index WHERE file_path = ?", (file_path,)
            )
            row = await cursor.fetchone()

            if row is None:
                return None

            # Convert to dict and parse JSON
            doc = dict(row)
            doc["metadata"] = json.loads(doc.get("metadata", "{}"))

            return doc

    async def get_document_by_id(self, doc_id: int) -> dict[str, Any] | None:
        """Retrieve document by ID.

        Args:
            doc_id: Document ID

        Returns:
            Document dict with parsed metadata, or None if not found
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute("SELECT * FROM document_index WHERE id = ?", (doc_id,))
            row = await cursor.fetchone()

            if row is None:
                return None

            # Convert to dict and parse JSON
            doc = dict(row)
            doc["metadata"] = json.loads(doc.get("metadata", "{}"))

            return doc

    async def mark_synced(self, doc_id: int) -> None:
        """Mark document as synced (embedding generation complete).

        Args:
            doc_id: Document ID

        Example:
            >>> await document_service.mark_synced(doc_id)
        """
        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    await conn.execute(
                        """
                        UPDATE document_index
                        SET sync_status = 'synced', last_synced_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
                        WHERE id = ?
                        """,
                        (doc_id,),
                    )
                    await conn.commit()
            except Exception:
                await conn.rollback()
                raise

    async def mark_failed(self, doc_id: int, error_message: str) -> None:
        """Mark document sync as failed with error message.

        Args:
            doc_id: Document ID
            error_message: Failure reason

        Example:
            >>> await document_service.mark_failed(doc_id, "Embedding API timeout")
        """
        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    # Store error in metadata
                    cursor = await conn.execute(
                        "SELECT metadata FROM document_index WHERE id = ?", (doc_id,)
                    )
                    row = await cursor.fetchone()

                    if row:
                        metadata = json.loads(row["metadata"])
                        metadata["last_error"] = error_message

                        await conn.execute(
                            """
                            UPDATE document_index
                            SET sync_status = 'failed', metadata = ?, updated_at = CURRENT_TIMESTAMP
                            WHERE id = ?
                            """,
                            (json.dumps(metadata), doc_id),
                        )
                        await conn.commit()
            except Exception:
                await conn.rollback()
                raise

    async def mark_stale(self, file_path: str, new_content: str) -> None:
        """Mark document as stale when content changes.

        Args:
            file_path: Document file path
            new_content: New content for hash generation

        Example:
            >>> await document_service.mark_stale("/docs/schema-design.md", new_content)
        """
        new_hash = hashlib.sha256(new_content.encode()).hexdigest()

        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    await conn.execute(
                        """
                        UPDATE document_index
                        SET content_hash = ?, sync_status = 'stale', updated_at = CURRENT_TIMESTAMP
                        WHERE file_path = ?
                        """,
                        (new_hash, file_path),
                    )
                    await conn.commit()
            except Exception:
                await conn.rollback()
                raise

    async def get_pending_documents(self, limit: int = 100) -> list[dict[str, Any]]:
        """Get documents needing embedding sync (pending or stale).

        Args:
            limit: Maximum results to return

        Returns:
            List of document dicts with parsed metadata

        Example:
            >>> pending_docs = await document_service.get_pending_documents()
            >>> for doc in pending_docs:
            ...     print(f"Syncing: {doc['file_path']}")
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT * FROM document_index
                WHERE sync_status IN ('pending', 'stale')
                ORDER BY created_at ASC
                LIMIT ?
                """,
                (limit,),
            )
            rows = await cursor.fetchall()

            # Parse JSON fields
            documents = []
            for row in rows:
                doc = dict(row)
                doc["metadata"] = json.loads(doc.get("metadata", "{}"))
                documents.append(doc)

            return documents

    async def search_by_type(self, document_type: str, limit: int = 50) -> list[dict[str, Any]]:
        """Search documents by type.

        Args:
            document_type: Document type filter
            limit: Maximum results to return

        Returns:
            List of document dicts

        Example:
            >>> design_docs = await document_service.search_by_type("design")
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT * FROM document_index
                WHERE document_type = ?
                ORDER BY created_at DESC
                LIMIT ?
                """,
                (document_type, limit),
            )
            rows = await cursor.fetchall()

            # Parse JSON fields
            documents = []
            for row in rows:
                doc = dict(row)
                doc["metadata"] = json.loads(doc.get("metadata", "{}"))
                documents.append(doc)

            return documents

    async def generate_and_store_embedding(
        self,
        document_id: int,
        content: str,
        namespace: str,
        file_path: str,
    ) -> int:
        """Generate embedding for document content and store in vector DB.

        Args:
            document_id: ID from document_index table
            content: Document text content to embed
            namespace: Document namespace
            file_path: Document file path

        Returns:
            rowid of inserted embedding

        Raises:
            ValueError: If document_id doesn't exist
            httpx.HTTPError: If embedding generation fails

        Example:
            >>> rowid = await service.generate_and_store_embedding(
            ...     document_id=1,
            ...     content="Memory management patterns...",
            ...     namespace="docs:architecture",
            ...     file_path="/docs/memory-architecture.md"
            ... )
        """
        import struct

        from abathur.services.embedding_service import EmbeddingService

        # Generate embedding
        embedding_service = EmbeddingService()
        embedding_vector = await embedding_service.generate_embedding(content)

        # Serialize to BLOB for vss0
        embedding_blob = struct.pack(f"{len(embedding_vector)}f", *embedding_vector)

        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    # Insert into vss0 virtual table
                    cursor = await conn.execute(
                        "INSERT INTO document_embeddings(rowid, embedding) VALUES(?, ?)",
                        (document_id, embedding_blob),
                    )
                    rowid = cursor.lastrowid
                    assert rowid is not None, "Failed to get rowid after vector insert"

                    # Insert metadata
                    await conn.execute(
                        """
                        INSERT INTO document_embedding_metadata
                        (rowid, document_id, namespace, file_path, embedding_model)
                        VALUES (?, ?, ?, ?, 'nomic-embed-text-v1.5')
                        """,
                        (rowid, document_id, namespace, file_path),
                    )

                    await conn.commit()
                    return int(rowid)
            except Exception:
                await conn.rollback()
                raise

    async def search_by_embedding(
        self, query_embedding: list[float], limit: int = 10, distance_threshold: float = 1000.0
    ) -> list[dict[str, Any]]:
        """Semantic search by embedding vector using L2 distance.

        Args:
            query_embedding: 768-dimensional query vector
            limit: Maximum results to return
            distance_threshold: Maximum L2 distance for results (default 1000.0, lower = more similar)

        Returns:
            List of matching documents with similarity scores, ordered by relevance

        Note:
            L2 distance on 768-dimensional embeddings typically ranges from 0 to ~1000+.
            Default threshold of 1000.0 captures most relevant results.

        Example:
            >>> results = await service.search_by_embedding(query_vec, limit=5)
            >>> for doc in results:
            ...     print(f"{doc['file_path']}: distance={doc['distance']}")
        """
        import struct

        # Serialize query vector to BLOB
        query_blob = struct.pack(f"{len(query_embedding)}f", *query_embedding)

        async with self.db._get_connection() as conn:
            # Use vss_search() for similarity search
            # sqlite-vss requires specific syntax with LIMIT in WHERE clause
            cursor = await conn.execute(
                """
                SELECT
                    m.document_id,
                    m.namespace,
                    m.file_path,
                    m.embedding_model,
                    m.created_at,
                    distance
                FROM (
                    SELECT rowid, distance
                    FROM document_embeddings
                    WHERE vss_search(embedding, ?)
                    LIMIT ?
                ) v
                INNER JOIN document_embedding_metadata m ON v.rowid = m.rowid
                WHERE distance <= ?
                ORDER BY distance
                """,
                (query_blob, limit, distance_threshold),
            )

            rows = await cursor.fetchall()

            results = []
            for row in rows:
                result = dict(row)
                results.append(result)

            return results

    async def semantic_search(
        self,
        query_text: str,
        namespace: str | None = None,
        limit: int = 10,
        distance_threshold: float = 1000.0,
    ) -> list[dict[str, Any]]:
        """Semantic search using natural language query.

        Args:
            query_text: Natural language search query
            namespace: Optional namespace filter
            limit: Maximum results
            distance_threshold: Max L2 distance (default 1000.0)

        Returns:
            List of matching documents with metadata

        Example:
            >>> results = await service.semantic_search(
            ...     "memory management patterns",
            ...     namespace="docs:architecture",
            ...     limit=5
            ... )
        """
        from abathur.services.embedding_service import EmbeddingService

        # Generate query embedding
        embedding_service = EmbeddingService()
        query_embedding = await embedding_service.generate_embedding(query_text)

        # Search by embedding
        results = await self.search_by_embedding(query_embedding, limit, distance_threshold)

        # Apply namespace filter if specified
        if namespace:
            results = [r for r in results if r["namespace"].startswith(namespace)]

        return results

    async def sync_document_to_vector_db(
        self, namespace: str, file_path: str, content: str
    ) -> dict[str, Any]:
        """Index document and generate embedding in one operation.

        Args:
            namespace: Document namespace
            file_path: Document file path
            content: Document content

        Returns:
            Dict with document_id and embedding_rowid

        Example:
            >>> result = await service.sync_document_to_vector_db(
            ...     namespace="docs:api",
            ...     file_path="/docs/memory_service.md",
            ...     content=markdown_content
            ... )
            >>> print(f"Document {result['document_id']} indexed")
        """
        # Extract title from content (first line or use file path)
        lines = content.split("\n")
        title = lines[0].strip("#").strip() if lines else file_path

        # Index document
        doc_id = await self.index_document(
            file_path=file_path,
            title=title,
            content=content,
            document_type="markdown",
            metadata={"indexed_for_search": True, "namespace": namespace},
        )

        # Generate and store embedding
        embedding_rowid = await self.generate_and_store_embedding(
            document_id=doc_id, content=content, namespace=namespace, file_path=file_path
        )

        # Mark as synced
        await self.mark_synced(doc_id)

        return {"document_id": doc_id, "embedding_rowid": embedding_rowid}

    async def list_all_documents(self, limit: int = 100) -> list[dict[str, Any]]:
        """List all indexed documents.

        Args:
            limit: Maximum results to return

        Returns:
            List of document dicts

        Example:
            >>> all_docs = await document_service.list_all_documents()
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT * FROM document_index
                ORDER BY created_at DESC
                LIMIT ?
                """,
                (limit,),
            )
            rows = await cursor.fetchall()

            # Parse JSON fields
            documents = []
            for row in rows:
                doc = dict(row)
                doc["metadata"] = json.loads(doc.get("metadata", "{}"))
                documents.append(doc)

            return documents
